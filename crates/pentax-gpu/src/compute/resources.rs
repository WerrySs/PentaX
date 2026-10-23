// SPDX-License-Identifier: GPL-2.0-only
//! Direct-resource specialization over one retained compiler analysis.
//! Descriptor facts confer no guest-memory or GPU authority.
//! Analyze first; only actual live direct user-SGPR descriptors are inspected.
use super::{ComputeState, Error as RegisterError, Limits};
use pentax_shader_compiler::{
    BufferAccess, BufferDescription, BufferRequirement, CompileError, CompiledShader,
    ComputeDescription, Limits as CompilerLimits, Profile, ShaderAnalysis, ShaderProgram,
    StageDescription, analyze,
};
use std::{fmt, mem::size_of};

#[derive(Debug)]
pub enum Error {
    Compiler(CompileError),
    Register(RegisterError),
    Unsupported(&'static str),
    Limit(&'static str),
    Allocation,
}

/// Conservative phase reservations and exact admitted aggregate descriptor bytes.
/// These must join the caller's decoder/session ledger once, before integration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub host_bytes: usize,
    pub work: u64,
    pub buffer_bytes: usize,
}

/// A descriptor fact, not checked guest backing or an uploaded storage buffer.
#[derive(Clone, Copy)]
pub struct BufferSpan {
    requirement: BufferRequirement,
    address: u64,
    bytes: usize,
}
impl BufferSpan {
    pub fn requirement(&self) -> BufferRequirement {
        self.requirement
    }
    pub fn address(&self) -> u64 {
        self.address
    }
    pub fn byte_count(&self) -> usize {
        self.bytes
    }
}
impl fmt::Debug for BufferSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferSpan")
            .field("requirement", &self.requirement)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// Owns the same analyzed IR and its finite compiler budget through finish.
pub struct Discovery {
    analysis: ShaderAnalysis,
    spans: Vec<BufferSpan>,
    descriptions: Vec<BufferDescription>,
    usage: Usage,
}
/// Compiled specialization plus metadata only. No input payload was read.
pub struct Specialization {
    shader: CompiledShader,
    spans: Vec<BufferSpan>,
    usage: Usage,
}
impl Discovery {
    pub fn buffers(&self) -> &[BufferSpan] {
        &self.spans
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn finish(self) -> Result<Specialization, Error> {
        let shader = self
            .analysis
            .finish(&self.descriptions)
            .map_err(Error::Compiler)?;
        let manifest = shader.manifest();
        if manifest.buffers().len() != self.spans.len()
            || manifest.shader_data_binding().is_some()
            || manifest.undefined_register_reads() != 0
            || !manifest.images().is_empty()
            || !manifest.samplers().is_empty()
        {
            return Err(Error::Unsupported(
                "compute resource specialization contract",
            ));
        }
        for (profile_index, (span, buffer)) in self.spans.iter().zip(manifest.buffers()).enumerate()
        {
            let requirement = span.requirement;
            if buffer.user_sgpr != requirement.user_sgpr {
                return Err(Error::Unsupported(
                    "compute resource specialization origin/order",
                ));
            }
            if buffer.read != requirement.read
                || buffer.written != requirement.written
                || buffer.scalar != requirement.scalar
                || buffer.coherent != requirement.coherent
                || buffer.profile_index != profile_index
                || buffer.max_byte_extent != requirement.max_byte_extent
                || buffer.first_use_pc != requirement.first_use_pc
            {
                return Err(Error::Unsupported("compute resource specialization access"));
            }
        }
        Ok(Specialization {
            shader,
            spans: self.spans,
            usage: self.usage,
        })
    }
}
impl Specialization {
    pub fn shader(&self) -> &CompiledShader {
        &self.shader
    }
    pub fn buffers(&self) -> &[BufferSpan] {
        &self.spans
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub(super) fn into_parts(self) -> (CompiledShader, Vec<BufferSpan>, Usage) {
        (self.shader, self.spans, self.usage)
    }
}

fn host(usage: &mut Usage, bytes: usize, limits: &Limits) -> Result<(), Error> {
    usage.host_bytes = usage
        .host_bytes
        .checked_add(bytes)
        .filter(|total| *total <= limits.max_host_bytes)
        .ok_or(Error::Limit("compute resource host bytes"))?;
    Ok(())
}
fn work(usage: &mut Usage, units: u64, limits: &Limits) -> Result<(), Error> {
    usage.work = usage
        .work
        .checked_add(units)
        .filter(|total| *total <= limits.max_work)
        .ok_or(Error::Limit("compute resource work"))?;
    Ok(())
}
fn reserve<T>(
    out: &mut Vec<T>,
    count: usize,
    usage: &mut Usage,
    limits: &Limits,
) -> Result<(), Error> {
    host(
        usage,
        count
            .checked_mul(size_of::<T>())
            .ok_or(Error::Limit("resource metadata"))?,
        limits,
    )?;
    out.try_reserve_exact(count)
        .map_err(|_| Error::Allocation)?;
    host(
        usage,
        out.capacity()
            .checked_sub(count)
            .and_then(|extra| extra.checked_mul(size_of::<T>()))
            .ok_or(Error::Limit("resource metadata capacity"))?,
        limits,
    )
}

/// The supplied program is already decoded under the caller's source budget.
/// The profile must have no predeclared resources. No Memory adapter is accepted:
/// addresses are descriptor facts and have not passed owner/permission checks.
pub fn discover(
    program: &ShaderProgram,
    description: ComputeDescription,
    registers: &ComputeState,
    limits: &Limits,
) -> Result<Discovery, Error> {
    discover_impl(program, description, registers, limits, true)
}

pub(super) fn discover_reserved(
    program: &ShaderProgram,
    description: ComputeDescription,
    registers: &ComputeState,
    limits: &Limits,
) -> Result<Discovery, Error> {
    discover_impl(program, description, registers, limits, false)
}

fn discover_impl(
    program: &ShaderProgram,
    description: ComputeDescription,
    registers: &ComputeState,
    limits: &Limits,
    reserve_phase: bool,
) -> Result<Discovery, Error> {
    let users = description.common.user_sgpr_count;
    if description.common.user_sgpr_base != 0 || users > 16 {
        return Err(Error::Unsupported("compute direct user register bank"));
    }
    if limits.compiler_phase_bytes == 0 || limits.compiler_phase_work == 0 {
        return Err(Error::Limit("compiler budget"));
    }
    let mut usage = Usage::default();
    // Standalone metadata callers reserve the conservative compiler phases.
    // The execution preparer has already reserved those exact phases once.
    if reserve_phase {
        host(
            &mut usage,
            limits
                .compiler_phase_bytes
                .checked_mul(3)
                .ok_or(Error::Limit("compiler bytes"))?,
            limits,
        )?;
        work(
            &mut usage,
            limits
                .compiler_phase_work
                .checked_mul(3)
                .ok_or(Error::Limit("compiler work"))?,
            limits,
        )?;
    }
    let compiler = CompilerLimits {
        max_bytes: limits.compiler_phase_bytes,
        max_work: limits.compiler_phase_work,
        max_spirv_words: limits.max_spirv_words,
        ..Default::default()
    };
    let profile = Profile::new(StageDescription::Compute(description))
        .map_err(|error| Error::Compiler(CompileError::Profile(error)))?;
    let analysis = analyze(program, profile, &compiler).map_err(Error::Compiler)?;
    if !analysis.images().is_empty() || !analysis.samplers().is_empty() {
        return Err(Error::Unsupported(
            "compute sampled resource backing is not implemented",
        ));
    }
    let count = analysis.buffers().len();
    // Each direct V# uses four aligned words in the actually retained16-word
    // bank. Larger compiler limits do not create additional queue registers.
    if count > usize::from(users / 4) {
        return Err(Error::Unsupported("compute direct buffer count"));
    }
    let mut spans = Vec::new();
    let mut descriptions = Vec::new();
    reserve(&mut spans, count, &mut usage, limits)?;
    reserve(&mut descriptions, count, &mut usage, limits)?;
    for &requirement in analysis.buffers() {
        if requirement.coherent
            || !requirement.user_sgpr.is_multiple_of(4)
            || requirement
                .user_sgpr
                .checked_add(4)
                .is_none_or(|end| end > users)
        {
            return Err(Error::Unsupported("compute direct buffer access/origin"));
        }
        work(&mut usage, 4, limits)?;
        let first = 0x240 + u32::from(requirement.user_sgpr);
        let word = |offset| registers.get(first + offset).map_err(Error::Register);
        let raw = [word(0)?, word(1)?, word(2)?, word(3)?];
        if raw[1] & 0xffff_0000 != 0 || raw[3] != 0 {
            return Err(Error::Unsupported(
                "compute raw unswizzled buffer descriptor",
            ));
        }
        let address = u64::from(raw[0]) | (u64::from(raw[1]) << 32);
        let bytes = raw[2] as usize;
        if address == 0
            || bytes == 0
            || !address.is_multiple_of(4)
            || !bytes.is_multiple_of(4)
            || address
                .checked_add(bytes as u64)
                .is_none_or(|end| end > 1 << 47)
            || bytes < requirement.max_byte_extent as usize
        {
            return Err(Error::Unsupported("compute direct buffer extent"));
        }
        usage.buffer_bytes = usage
            .buffer_bytes
            .checked_add(bytes)
            .filter(|total| *total <= limits.max_buffer_bytes)
            .ok_or(Error::Limit("compute aggregate buffer bytes"))?;
        let access = match (requirement.read, requirement.written) {
            (true, false) => BufferAccess::ReadOnly,
            (false, true) => BufferAccess::WriteOnly,
            (true, true) => BufferAccess::ReadWrite,
            (false, false) => return Err(Error::Unsupported("compute buffer without live access")),
        };
        spans.push(BufferSpan {
            requirement,
            address,
            bytes,
        });
        descriptions.push(BufferDescription {
            user_sgpr: requirement.user_sgpr,
            stride: 0,
            swizzle: false,
            index_stride: 0,
            access,
        });
    }
    Ok(Discovery {
        analysis,
        spans,
        descriptions,
        usage,
    })
}

#[cfg(test)]
mod tests;
