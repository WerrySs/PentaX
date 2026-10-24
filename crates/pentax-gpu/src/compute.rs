// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded compute packet/resource preparation; see NOTICE.

//! Flat compute preparation and bounded stateful command plans without GPU writes.
//! Register fields follow the pinned Kyty compute paths, not inferred reset state.
pub use crate::marker::{Marker, MarkerKind};

use crate::Header;
pub use crate::memory_sync::{Synchronization, SynchronizationKind};
pub use crate::memory_transfer::Transfer;
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{
    CommonDescription, CompiledShader, ComputeDescription, Limits as CompilerLimits, ShaderProgram,
    SubgroupProfile, WaveSize,
};
use std::fmt;

mod commands;
mod ordered;
pub mod resources;
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
    /// Admit every descriptor span before any resource payload is fetched.
    /// Native adapters enforce current owner, access and physical exclusions.
    fn preflight_buffers(
        &self,
        _buffers: &[resources::BufferSpan],
        _work: u64,
    ) -> Result<u64, Error> {
        Ok(0)
    }
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
    buffers: Vec<PreparedBuffer>,
    push: [u8; 128],
    groups: [u32; 3],
    thread_extent: Option<[u32; 3]>,
    resource_controls: ResourceControls,
    usage: Usage,
}
/// Named raw R1 compatibility metadata; never an initialized SGPR count or authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceControls {
    legacy_sgprs: u8,
    mem_ordered: bool,
}
impl ResourceControls {
    pub fn legacy_sgprs(self) -> u8 {
        self.legacy_sgprs
    }
    pub fn mem_ordered(self) -> bool {
        self.mem_ordered
    }
    fn decode(r1: u32) -> Result<Self, Error> {
        const VGPRS: u32 = 0x3f;
        const LEGACY_SGPRS: u32 = 0xf << 6;
        const MEM_ORDERED: u32 = 1 << 30;
        // Public receiver compatibility for the legacy allocation nibble does
        // not seed USER registers or increase the compiler's normal SGPR bank.
        // All remaining numeric, clamp, IEEE, debug and unknown fields stay exact.
        if r1 & !(VGPRS | LEGACY_SGPRS | MEM_ORDERED) != 0x2c_0000 {
            return Err(Error::Unsupported("compute resource configuration"));
        }
        Ok(Self {
            legacy_sgprs: ((r1 & LEGACY_SGPRS) >> 6) as u8,
            mem_ordered: r1 & MEM_ORDERED != 0,
        })
    }
    fn check_memory_domain(self, shader: &CompiledShader) -> Result<(), Error> {
        // MEM_ORDERED distinguishes load/return-atomic versus SAMPLE completion
        // classes. Actual discovery already refuses compute images/samplers;
        // keep that closed domain explicit at this field's acceptance boundary.
        // Neither a host fence nor an added barrier establishes this contract.
        let manifest = shader.manifest();
        if self.mem_ordered
            && (!manifest.images().is_empty()
                || !manifest.samplers().is_empty()
                || manifest.required_features().image_query())
        {
            return Err(Error::Unsupported("compute MEM_ORDERED sampled class"));
        }
        Ok(())
    }
}

/// One actual manifest descriptor-array element and owned preserved input.
/// Construction is private; addresses never confer guest-memory authority.
pub struct PreparedBuffer {
    address: u64,
    input: Vec<u8>,
    read: bool,
    written: bool,
}
impl PreparedBuffer {
    pub fn address(&self) -> u64 {
        self.address
    }
    pub fn input(&self) -> &[u8] {
        &self.input
    }
    pub fn read(&self) -> bool {
        self.read
    }
    pub fn written(&self) -> bool {
        self.written
    }
}
impl fmt::Debug for PreparedCompute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedCompute")
            .field("buffers", &self.buffers.len())
            .field("groups", &self.groups)
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}
impl PreparedCompute {
    pub fn shader(&self) -> &CompiledShader {
        &self.shader
    }
    pub fn buffers(&self) -> &[PreparedBuffer] {
        &self.buffers
    }
    /// Checked legacy single-resource view; zero/multiple resources return None.
    pub fn input(&self) -> Option<&[u8]> {
        if let [buffer] = self.buffers.as_slice() {
            Some(buffer.input())
        } else {
            None
        }
    }
    pub fn push_data(&self) -> &[u8; 128] {
        &self.push
    }
    pub fn groups(&self) -> [u32; 3] {
        self.groups
    }
    /// Original thread counts, distinct from ceil-rounded host workgroup counts.
    pub fn thread_extent(&self) -> Option<[u32; 3]> {
        self.thread_extent
    }
    pub fn resource_controls(&self) -> ResourceControls {
        self.resource_controls
    }

    pub fn buffer_address(&self) -> Option<u64> {
        if let [buffer] = self.buffers.as_slice() {
            Some(buffer.address)
        } else {
            None
        }
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn writes_buffer(&self) -> bool {
        self.buffers.iter().any(|buffer| buffer.written)
    }
    pub fn retained_host_bytes(&self) -> Result<usize, Error> {
        let mut bytes = self
            .shader
            .retained_host_bytes()
            .checked_add(std::mem::size_of::<Self>())
            .and_then(|bytes| {
                bytes.checked_add(
                    self.buffers
                        .capacity()
                        .checked_mul(std::mem::size_of::<PreparedBuffer>())?,
                )
            })
            .ok_or(Error::Limit("retained compute bytes"))?;
        for buffer in &self.buffers {
            bytes = bytes
                .checked_add(buffer.input.capacity())
                .ok_or(Error::Limit("retained compute bytes"))?;
        }
        Ok(bytes)
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
    values: [Option<u32>; 29],
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
            values: [None; 29],
            indirect_base: None,
        }
    }
    /// Fresh queue state with the hardware's zero start coordinates. Detached
    /// command decoding continues to use `empty()` and requires explicit state.
    /// Other registers stay unknown until established by an admitted command.
    pub const fn for_new_queue() -> Self {
        let mut state = Self::empty();
        state.values[0] = Some(0);
        state.values[1] = Some(0);
        state.values[2] = Some(0);
        state
    }
    // Only defaults with an exact public field/register correspondence are
    // restored. Packed raw resource controls remain unknown.
    pub(crate) fn reset(&mut self) {
        *self = Self::for_new_queue();
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
            // Public HwShSetCsRegister treats these exact registers as scheduling
            // hints. Retain their opaque values in queue/dispatch snapshots;
            // serialized host execution does not adopt guest occupancy or pace
            // as host quotas, memory authority or shader resource configuration.
            0x215 => 27,
            0x22a => 28,
            _ => return None,
        })
    }
    fn get(&self, reg: u32) -> Result<u32, Error> {
        Self::index(reg)
            .and_then(|i| self.values[i])
            .ok_or(Error::MissingRegister(reg))
    }
    pub(crate) fn set_dispatch_base(&mut self, words: &[u32]) -> Result<(), Error> {
        if words.len() != 4 || words[0] != 0xc002_1102 || words[1] & 15 != 1 {
            return Err(Error::Unsupported("compute indirect base packet"));
        }
        let address = u64::from(words[2] & !7) | (u64::from(words[3] & 0xffff) << 32);
        if address >= 1 << 47 {
            return Err(Error::Unsupported("indirect base address"));
        }
        self.indirect_base = Some(address);
        Ok(())
    }
    /// Shared offset routing; this exposes no register or memory authority.
    pub(crate) fn admits_shader_register(reg: u32) -> bool {
        Self::index(reg).is_some()
    }
    pub(crate) fn write(&mut self, reg: u32, value: u32) -> Result<(), Error> {
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

const USE_THREAD_DIMENSIONS: u32 = 1 << 5;

// Exact public CS_W32_EN selects the guest wave width. The independent thread
// dimension bit selects checked geometry and runtime initial EXEC. Other controls
// retain the existing bounded profile; no subgroup width is inferred from host.
fn dispatch_wave_size(mode: u32) -> Result<WaveSize, Error> {
    match mode & !USE_THREAD_DIMENSIONS {
        0x0001 | 0x0005 | 0x0041 => Ok(WaveSize::Wave64),
        0x8001 | 0x8005 | 0x8041 => Ok(WaveSize::Wave32),
        _ => Err(Error::Unsupported("compute dispatch initiator")),
    }
}

fn dispatch_geometry(
    dimensions: [u32; 3],
    local: [u32; 3],
    thread_dimensions: bool,
    maximum: u64,
) -> Result<([u32; 3], u64), Error> {
    let mut groups = dimensions;
    if thread_dimensions {
        for axis in 0..3 {
            let size = local[axis];
            if size == 0 {
                return Err(Error::Limit("dispatch local geometry"));
            }
            // div_ceil avoids the overflowing (threads + size - 1) formula.
            // A nonzero divisor and u32 thread count guarantee its u32 result.
            groups[axis] = dimensions[axis].div_ceil(size);
            // GlobalInvocationId is u32. Reject padded axis wrap before any
            // shader/resource read, even when a caller sets an enormous quota.
            groups[axis]
                .checked_mul(size)
                .ok_or(Error::Limit("dispatch thread extent"))?;
        }
    }
    let mut invocations = 1u64;
    for value in groups.into_iter().chain(local) {
        invocations = invocations
            .checked_mul(u64::from(value))
            .filter(|value| *value != 0 && *value <= maximum)
            .ok_or(Error::Limit("dispatch invocations"))?;
    }
    Ok((groups, invocations))
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
    dispatch_wave_size(dispatch.mode)?;
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
    let wave_size = dispatch_wave_size(mode)?;
    if mode & 4 == 0 {
        for reg in 0x204..=0x206 {
            if regs.get(reg)? != 0 {
                return Err(Error::Unsupported("compute start coordinates"));
            }
        }
    }
    let local = [regs.get(0x207)?, regs.get(0x208)?, regs.get(0x209)?];
    let thread_dimensions = mode & USE_THREAD_DIMENSIONS != 0;
    let thread_extent = thread_dimensions.then_some(groups);
    let (groups, invocations) =
        dispatch_geometry(groups, local, thread_dimensions, limits.max_invocations)?;
    budget.usage.invocations = invocations;
    let r1 = regs.get(0x212)?;
    let r2 = regs.get(0x213)?;
    let resource_controls = ResourceControls::decode(r1)?;
    if r2 & !(0x1fbe | 0x00ff_8000) != 0 || regs.get(0x228)? != 0 {
        return Err(Error::Unsupported("compute resource configuration"));
    }
    // Public GFX10.3 COMPUTE_PGM_RSRC2.LDS_SIZE is a nine-bit count of
    // 512-byte allocation units. All other scratch/trap/exception bits keep
    // the existing refusal; the compiler owns the admitted LDS operations.
    let shared_memory_bytes = ((r2 >> 15) & 0x1ff) * 512;
    if shared_memory_bytes > 65_536 {
        return Err(Error::Unsupported("compute LDS allocation"));
    }
    let user_count = ((r2 >> 1) & 31) as u8;
    let tid = ((r2 >> 11) & 3) as u8;
    if user_count > 16 || tid > 2 {
        return Err(Error::Unsupported("compute initial registers"));
    }
    let lo = regs.get(0x20c)?;
    let hi = regs.get(0x20d)?;
    if hi > 255 {
        return Err(Error::Unsupported("shader address high bits"));
    }
    let code_address = (u64::from(lo) << 8) | (u64::from(hi) << 40);
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
    let mut common = CommonDescription::new(wave_size);
    common.subgroup = Some(SubgroupProfile {
        size: wave_size.lanes(),
    });
    common.user_sgpr_count = user_count;
    let description = ComputeDescription {
        common,
        workgroup_size: local,
        thread_ids: tid + 1,
        group_id: [r2 & 0x80 != 0, r2 & 0x100 != 0, r2 & 0x200 != 0],
        workgroup_register: user_count,
        tg_size: r2 & 0x400 != 0,
        float_mode: 0xc0,
        shared_memory_bytes,
        dispatch_thread_dimensions: thread_dimensions,
    };
    let resource_limits = Limits {
        max_host_bytes: limits
            .max_host_bytes
            .checked_sub(budget.usage.host_bytes)
            .ok_or(Error::Limit("resource host bytes"))?,
        max_work: limits
            .max_work
            .checked_sub(budget.usage.work)
            .ok_or(Error::Limit("resource work"))?,
        ..*limits
    };
    let map_error = |error| match error {
        resources::Error::Compiler(_) => Error::Compile,
        resources::Error::Register(error) => error,
        resources::Error::Unsupported("compute raw unswizzled buffer descriptor") => {
            Error::Unsupported("raw unswizzled buffer descriptor")
        }
        resources::Error::Unsupported("compute direct buffer extent") => {
            Error::Unsupported("buffer extent")
        }
        resources::Error::Unsupported(reason) => Error::Unsupported(reason),
        resources::Error::Limit(reason) => Error::Limit(reason),
        resources::Error::Allocation => Error::Allocation,
    };
    let discovery = resources::discover_reserved(&program, description, &regs, &resource_limits)
        .map_err(map_error)?;
    let (compiled, spans, resource_usage) = discovery.finish().map_err(map_error)?.into_parts();
    budget.host(resource_usage.host_bytes)?;
    budget.work(resource_usage.work)?;
    resource_controls.check_memory_domain(&compiled)?;
    let manifest = compiled.manifest();
    let preflight =
        memory.preflight_buffers(&spans, limits.max_work.saturating_sub(budget.usage.work))?;
    budget.work(preflight)?;
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
        // Every element of the one descriptor array is uploaded at its exact
        // guest base; all packed buffer-byte offsets stay explicitly zero.
        if layout.memory_offset_count as usize != spans.len()
            || layout
                .start_dword
                .checked_add(layout.memory_offset_dword)
                .and_then(|at| at.checked_add(layout.memory_offset_count.div_ceil(4)))
                .is_none_or(|end| end > 32)
        {
            return Err(Error::Unsupported("compute buffer offset layout"));
        }
    }
    if let Some(relative) = manifest.dispatch_thread_extent_dword() {
        let extent = thread_extent.ok_or(Error::Unsupported("compute thread extent mode"))?;
        let layout = manifest
            .push_data()
            .ok_or(Error::Unsupported("compute thread extent layout"))?;
        let end = relative
            .checked_add(3)
            .ok_or(Error::Unsupported("compute thread extent layout"))?;
        if relative < layout.user_sgprs.len() as u32 || end > layout.memory_offset_dword {
            return Err(Error::Unsupported("compute thread extent overlap"));
        }
        let start = layout
            .start_dword
            .checked_add(relative)
            .and_then(|value| value.checked_mul(4))
            .ok_or(Error::Unsupported("compute thread extent layout"))?
            as usize;
        let output = push
            .get_mut(
                start
                    ..start
                        .checked_add(12)
                        .ok_or(Error::Unsupported("compute thread extent layout"))?,
            )
            .ok_or(Error::Unsupported("compute thread extent layout"))?;
        for (word, value) in output.chunks_exact_mut(4).zip(extent) {
            word.copy_from_slice(&value.to_le_bytes());
        }
    }
    budget.host(
        spans
            .len()
            .checked_mul(std::mem::size_of::<PreparedBuffer>())
            .ok_or(Error::Limit("resource metadata"))?,
    )?;
    let mut buffers = Vec::new();
    buffers
        .try_reserve_exact(spans.len())
        .map_err(|_| Error::Allocation)?;
    budget.host((buffers.capacity() - spans.len()) * std::mem::size_of::<PreparedBuffer>())?;
    // Store in actual compiler descriptor-array order, not register order.
    for (element, span) in manifest.buffers().iter().zip(&spans) {
        // Discovery.finish checked exact source first-use order and extent.
        // Upload order is the compiler array order, never SGPR sorting.
        if span.requirement().user_sgpr != element.user_sgpr {
            return Err(Error::Unsupported("compute buffer source order"));
        }
        let length = span.byte_count();
        budget.host(length)?;
        let mut input = Vec::new();
        input
            .try_reserve_exact(length)
            .map_err(|_| Error::Allocation)?;
        budget.host(input.capacity() - length)?;
        budget.work(length as u64)?;
        input.resize(length, 0);
        memory
            .snapshot_buffer(span.address(), &mut input, element.read, element.written)
            .map_err(Error::Memory)?;
        buffers.push(PreparedBuffer {
            address: span.address(),
            input,
            read: element.read,
            written: element.written,
        });
    }
    Ok(PreparedCompute {
        shader: compiled,
        buffers,
        push,
        groups,
        thread_extent,
        resource_controls,
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
#[cfg(test)]
#[path = "compute/transfer_tests.rs"]
mod transfer_tests;

#[cfg(test)]
#[path = "compute/hint_tests.rs"]
mod hint_tests;

#[cfg(test)]
#[path = "compute/wave_tests.rs"]
mod wave_tests;

#[cfg(test)]
#[path = "compute/thread_dimensions_tests.rs"]
mod thread_dimensions_tests;
