// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit register/profile admission and owned resources.

use super::linkage::{PixelRegisters, validate_shader_interfaces};
use super::{
    Budget, DecodedTarget, Draw, Error, GraphicsState, Limits, Memory, PreparedBuffer,
    PreparedDraw, PreparedIndices, RegisterSpace, TargetDecodeLimits, TargetGeometry, TargetLayout,
    Usage,
};
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{
    BufferAccess, BufferDescription, CommonDescription, CompiledShader, ImageDescription,
    Limits as CompilerLimits, Profile, SamplerDescription, ShaderProgram, StageDescription,
    VertexDescription, WaveSize, analyze,
};

fn context(state: &GraphicsState, register: u32) -> Result<u32, Error> {
    state.get(RegisterSpace::Context, register)
}
fn sh(state: &GraphicsState, register: u32) -> Result<u32, Error> {
    state.get(RegisterSpace::Sh, register)
}
fn expected(state: &GraphicsState, register: u32, value: u32) -> Result<(), Error> {
    if context(state, register)? != value {
        return Err(Error::Unsupported("graphics fixed-function state"));
    }
    Ok(())
}

struct Configuration {
    address: u64,
    width: u32,
    height: u32,
    bytes: usize,
    geometry: TargetGeometry,
    vertex_address: u64,
    pixel_address: u64,
    vertex_users: u8,
    pixel_users: u8,
    vertex_wave: WaveSize,
    pixel: PixelRegisters,
}

fn shader_address(state: &GraphicsState, low: u32) -> Result<u64, Error> {
    let lo = sh(state, low)?;
    let hi = sh(state, low + 1)?;
    if hi > 255 {
        return Err(Error::Unsupported("shader address high bits"));
    }
    let address = (u64::from(lo) << 8) | (u64::from(hi) << 40);
    if address == 0 || address >= 1 << 47 {
        return Err(Error::Unsupported("shader address"));
    }
    Ok(address)
}

fn configuration(draw: &Draw, budget: &mut Budget<'_>) -> Result<Configuration, Error> {
    // Fixed, bounded register visits before allocation or any resource read.
    budget.work(128)?;
    let state = &draw.state;
    if state.get(RegisterSpace::Uconfig, 0x242)? != 4 {
        return Err(Error::Unsupported("triangle-list primitive"));
    }
    if state.get(RegisterSpace::Uconfig, 0x24a)? != 0 {
        return Err(Error::Unsupported("draw-state vertex/instance offsets"));
    }
    if draw.indices.is_some() && state.get(RegisterSpace::Uconfig, 0x24b)? != 0 {
        return Err(Error::Unsupported("indexed primitive restart"));
    }
    let vertices = u64::from(draw.vertex_count)
        .checked_mul(u64::from(draw.instance_count))
        .filter(|n| *n != 0 && *n <= budget.limits.max_vertices)
        .ok_or(Error::Limit("draw vertices"))?;
    budget.usage.vertices = vertices;
    // No DCC/CMASK/FMASK, layered views, depth, blending, logic operations,
    // clipping overrides, raster discard, extra viewports or sample shading.
    for register in [
        0x80, 0x1c4, 0x1e0, 0x200, 0x203, 0x205, 0x207, 0x293, 0x2f8, 0x31b, 0x31d, 0x31e, 0x31f,
        0x321, 0x325, 0x398, 0x3a0, 0x3a8,
    ] {
        expected(state, register, 0)?;
    }
    for (register, value) in [
        (0x8e, 0xf),
        (0x8f, 0xf),
        (0x1c3, 4),
        (0x1c5, 9),
        (0x202, 0x00cc_0010),
        (0x204, 1 << 19),
        (0x206, 0x3f),
        (0x292, 2),
        // PAL's UNorm path sets BLEND_CLAMP and ROUND_BY_HALF (enum0).
        // CB RESOURCE_TYPE uses ImageType::Tex2d=1, not SQ image descriptor9.
        (0x31c, (10 << 2) | (1 << 15)),
    ] {
        expected(state, register, value)?;
    }
    let dimensions = context(state, 0x3b0)?;
    if dimensions >> 28 != 0 {
        return Err(Error::Unsupported("render-target mip levels"));
    }
    let width = ((dimensions >> 14) & 0x3fff) + 1;
    let height = (dimensions & 0x3fff) + 1;
    if width > budget.limits.max_dimension || height > budget.limits.max_dimension {
        return Err(Error::Limit("render-target dimensions"));
    }
    // Retain the existing Vulkan-visible width profile. Tiled pitch is a
    // separate padded guest extent and never changes this logical viewport.
    if !width.is_multiple_of(64) {
        return Err(Error::Unsupported("RGBA8 visible width profile"));
    }
    // Public pm4.h CB_COLOR0_ATTRIB3 decodes COLOR_SW_MODE at14, type at24.
    // kRenderTarget is0x1b. Exact words reject depth/layers, pipe flags and VRS;
    // no unproved swizzle or compression bit is masked away.
    let (layout, pitch, padded_height) = match context(state, 0x3b8)? {
        0x0100_0000 => (TargetLayout::LinearRgba8, width, height),
        0x0106_c000 => (
            TargetLayout::RenderTarget64KbRgba8,
            width.next_multiple_of(128),
            height.next_multiple_of(128),
        ),
        _ => return Err(Error::Unsupported("render-target swizzle profile")),
    };
    let extent = width | (height << 16);
    for (tl, br, top_left) in [
        (0x0c, 0x0d, 0),
        (0x81, 0x82, 0x8000_0000),
        (0x90, 0x91, 0x8000_0000),
        (0x94, 0x95, 0x8000_0000),
    ] {
        expected(state, tl, top_left)?;
        expected(state, br, extent)?;
    }
    for (register, value) in [
        (0x10f, width as f32 / 2.0),
        (0x110, width as f32 / 2.0),
        (0x111, height as f32 / 2.0),
        (0x112, height as f32 / 2.0),
        (0x113, 1.0),
        (0x114, 0.0),
    ] {
        expected(state, register, value.to_bits())?;
    }
    let tight_bytes = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .ok()
        .filter(|n| *n <= budget.limits.max_target_bytes)
        .ok_or(Error::Limit("render-target bytes"))?;
    let bytes = usize::try_from(u64::from(pitch) * u64::from(padded_height) * 4)
        .ok()
        .filter(|n| *n <= budget.limits.max_target_bytes)
        .ok_or(Error::Limit("padded render-target bytes"))?;
    let high = context(state, 0x390)?;
    if high > 255 {
        return Err(Error::Unsupported("render-target address high bits"));
    }
    let address = (u64::from(context(state, 0x318)?) << 8) | (u64::from(high) << 40);
    if layout == TargetLayout::RenderTarget64KbRgba8 && !address.is_multiple_of(65536) {
        return Err(Error::Unsupported("render-target block alignment"));
    }
    if address == 0
        || address
            .checked_add(bytes as u64)
            .is_none_or(|n| n > 1 << 47)
    {
        return Err(Error::Unsupported("render-target extent"));
    }
    let stages = context(state, 0x2d5)?;
    if stages & !0x0040_0000 != 0 || sh(state, 0x88)? != 0 || sh(state, 0x89)? != 0 {
        return Err(Error::Unsupported(
            "merged/NGG/geometry/tessellation stages",
        ));
    }
    state.primitive_profile()?;
    if sh(state, 0x8a)? & !0x3f != 0x602c_0000 || sh(state, 0x0a)? & !0x3f != 0x002c_0000 {
        return Err(Error::Unsupported("graphics shader resources1"));
    }
    let vertex_resources = sh(state, 0x8b)?;
    let pixel_resources = sh(state, 0x0b)?;
    if vertex_resources & !0x3e != 0x0003_0000 || pixel_resources & !0x3e != 0 {
        return Err(Error::Unsupported("graphics shader scratch/LDS/resources2"));
    }
    let vertex_users = ((vertex_resources >> 1) & 31) as u8;
    let pixel_users = ((pixel_resources >> 1) & 31) as u8;
    if vertex_users > 16 || pixel_users > 16 {
        return Err(Error::Unsupported("graphics user SGPR count"));
    }
    Ok(Configuration {
        pixel: PixelRegisters::read(state, budget)?,
        address,
        width,
        height,
        bytes,
        geometry: TargetGeometry {
            layout,
            width,
            height,
            pitch,
            guest_bytes: bytes,
            tight_bytes,
        },
        vertex_address: shader_address(state, 0xc8)?,
        pixel_address: shader_address(state, 0x08)?,
        vertex_users,
        pixel_users,
        vertex_wave: if stages & 0x0040_0000 != 0 {
            WaveSize::Wave32
        } else {
            WaveSize::Wave64
        },
    })
}

fn read_program(
    memory: &impl Memory,
    address: u64,
    stage: ShaderStage,
    budget: &mut Budget<'_>,
    compiler: &CompilerLimits,
) -> Result<ShaderProgram, Error> {
    let count = budget.limits.max_shader_words;
    if count == 0 {
        return Err(Error::Limit("shader words"));
    }
    budget.work(
        (count as u64)
            .checked_mul(count as u64)
            .and_then(|n| n.checked_mul(64))
            .ok_or(Error::Limit("shader walking work"))?,
    )?;
    let reserved = count.checked_mul(4).ok_or(Error::Limit("shader words"))?;
    budget.host(reserved)?;
    let mut words = Vec::new();
    words
        .try_reserve_exact(count)
        .map_err(|_| Error::Allocation)?;
    budget.host(
        (words.capacity() - count)
            .checked_mul(4)
            .ok_or(Error::Limit("shader words"))?,
    )?;
    let word_bytes = words
        .capacity()
        .checked_mul(4)
        .ok_or(Error::Limit("shader words"))?;
    for index in 0..count {
        let at = (index as u64)
            .checked_mul(4)
            .and_then(|n| address.checked_add(n))
            .filter(|n| n.checked_add(4).is_some_and(|end| end <= 1 << 47))
            .ok_or(Error::Unsupported("shader extent"))?;
        let mut bytes = [0; 4];
        memory.read(at, &mut bytes).map_err(Error::Memory)?;
        let word = u32::from_le_bytes(bytes);
        words.push(word);
        // An ENDPGM-shaped literal is not a terminator unless the independent
        // instruction decoder accepts the complete source at this boundary.
        if word == 0xbf81_0000 {
            // Decode owns a source/record span while validation owns its
            // independently bounded tables. Failed attempts drop both before
            // the next attempt. The source copy has a separate word cap.
            let phase = compiler
                .max_bytes
                .checked_add(compiler.decode.max_record_bytes)
                .and_then(|n| reserved.checked_add(n))
                .ok_or(Error::Limit("compiler bytes"))?;
            budget.peak(phase)?;
            let Ok(program) = ShaderProgram::decode(&words, stage, compiler) else {
                continue;
            };
            if program.instructions().any(|i| {
                i.branch()
                    .is_some_and(|b| b.target_byte_offset() <= i64::from(i.byte_offset()))
            }) {
                return Err(Error::Unsupported("unbounded graphics shader loop"));
            }
            budget.usage.shader_words = budget
                .usage
                .shader_words
                .checked_add(words.len())
                .ok_or(Error::Limit("shader words"))?;
            budget.host(program.retained_host_bytes())?;
            drop(words);
            budget.release(word_bytes)?;
            return Ok(program);
        }
    }
    Err(Error::Limit("terminated shader words"))
}

fn validate_manifest(compiled: &CompiledShader) -> Result<(), Error> {
    let manifest = compiled.manifest();
    let features = manifest.required_features();
    if manifest.shader_data_binding().is_some()
        || manifest.undefined_register_reads() != 0
        || manifest.required_subgroup_size().is_some()
        || features.subgroup_basic()
        || features.subgroup_ballot()
        || features.draw_parameters()
    {
        return Err(Error::Unsupported("graphics resource/subgroup manifest"));
    }
    if manifest.images().len() > 16
        || manifest.samplers().len() > 16
        || manifest.stage() != ShaderStage::Pixel
            && (!manifest.images().is_empty() || !manifest.samplers().is_empty())
        || manifest.images().iter().enumerate().any(|(index, image)| {
            image.profile_index != index
                || image.binding != 58 + index as u32
                || image.user_sgpr > 8
                || !image.user_sgpr.is_multiple_of(4)
        })
        || manifest
            .samplers()
            .iter()
            .enumerate()
            .any(|(index, sampler)| {
                sampler.profile_index != index
                    || sampler.binding != 74 + index as u32
                    || sampler.user_sgpr > 12
                    || !sampler.user_sgpr.is_multiple_of(4)
            })
    {
        return Err(Error::Unsupported("graphics sampled resource manifest"));
    }
    let buffers = manifest.buffers();
    let binding = if manifest.stage() == ShaderStage::Pixel {
        57
    } else {
        0
    };
    if buffers.len() > 1
        || manifest.buffer_binding() != (!buffers.is_empty()).then_some(binding)
        || buffers.iter().any(|buffer| {
            !buffer.read || buffer.written || buffer.coherent || buffer.profile_index != 0
        })
    {
        return Err(Error::Unsupported("graphics read-only buffer manifest"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct BufferSource {
    range: crate::compute::CommandRange,
}

#[derive(Default)]
struct SampledSources {
    images: Vec<crate::image_descriptor::Image2D>,
    samplers: Vec<crate::image_descriptor::Sampler>,
}

fn compile_stage(
    program: &ShaderProgram,
    profile: Profile,
    draw: &Draw,
    vertex: bool,
    compiler: &CompilerLimits,
    budget: &mut Budget<'_>,
) -> Result<(CompiledShader, Option<BufferSource>, SampledSources), Error> {
    let state = &draw.state;
    let users = profile.common().user_sgpr_count;
    let native = vertex && draw.native_offsets.is_some();
    budget.peak(compiler.max_bytes)?;
    let analysis = if native {
        let StageDescription::Vertex(description) = profile.description() else {
            return Err(Error::Compile);
        };
        pentax_shader_compiler::analyze_native_indirect_vertex(
            program,
            description.clone(),
            compiler,
        )
    } else {
        analyze(program, profile, compiler)
    }
    .map_err(|_| Error::Compile)?;
    let mut description = None;
    let source = if let [requirement] = analysis.buffers() {
        if !requirement.read
            || requirement.written
            || requirement.coherent
            || !requirement.user_sgpr.is_multiple_of(4)
            || requirement
                .user_sgpr
                .checked_add(4)
                .is_none_or(|end| end > users)
        {
            return Err(Error::Unsupported("graphics direct read-only buffer"));
        }
        budget.work(4)?;
        let first = if vertex { 0x8c } else { 0x0c } + u32::from(requirement.user_sgpr);
        let descriptor = [
            sh(state, first)?,
            sh(state, first + 1)?,
            sh(state, first + 2)?,
            sh(state, first + 3)?,
        ];
        // Same explicit raw shape as the checked compute path: no stride,
        // swizzle, formatted selectors, add-TID, or ignored descriptor flags.
        if descriptor[1] & 0xffff_0000 != 0 || descriptor[3] != 0 {
            return Err(Error::Unsupported(
                "graphics raw unswizzled buffer descriptor",
            ));
        }
        let address = u64::from(descriptor[0]) | (u64::from(descriptor[1]) << 32);
        let size = u64::from(descriptor[2]);
        if address == 0
            || size == 0
            || !address.is_multiple_of(4)
            || !size.is_multiple_of(4)
            || address.checked_add(size).is_none_or(|end| end > 1 << 47)
        {
            return Err(Error::Unsupported("graphics buffer extent"));
        }
        description = Some(BufferDescription {
            user_sgpr: requirement.user_sgpr,
            stride: 0,
            swizzle: false,
            index_stride: 0,
            access: BufferAccess::ReadOnly,
        });
        Some(BufferSource {
            range: crate::compute::CommandRange { address, size },
        })
    } else if analysis.buffers().is_empty() {
        None
    } else {
        return Err(Error::Unsupported("graphics buffer count"));
    };
    if analysis.images().len() > 16
        || analysis.samplers().len() > 16
        || vertex && (!analysis.images().is_empty() || !analysis.samplers().is_empty())
    {
        return Err(Error::Unsupported("pixel sampled resource count/stage"));
    }
    let mut sampled = SampledSources::default();
    let mut images = Vec::new();
    let mut samplers = Vec::new();
    let initial_description_bytes = analysis.images().len()
        * std::mem::size_of::<ImageDescription>()
        + analysis.samplers().len() * std::mem::size_of::<SamplerDescription>();
    let initial_image_bytes =
        analysis.images().len() * std::mem::size_of::<crate::image_descriptor::Image2D>();
    let initial_sampler_bytes =
        analysis.samplers().len() * std::mem::size_of::<crate::image_descriptor::Sampler>();
    budget.peak(
        compiler
            .max_bytes
            .checked_add(initial_description_bytes)
            .and_then(|n| n.checked_add(initial_image_bytes))
            .and_then(|n| n.checked_add(initial_sampler_bytes))
            .ok_or(Error::Limit("sampled compiler metadata"))?,
    )?;
    budget.host(initial_description_bytes)?;
    budget.host(initial_image_bytes)?;
    budget.host(initial_sampler_bytes)?;
    sampled
        .images
        .try_reserve_exact(analysis.images().len())
        .map_err(|_| Error::Allocation)?;
    sampled
        .samplers
        .try_reserve_exact(analysis.samplers().len())
        .map_err(|_| Error::Allocation)?;
    images
        .try_reserve_exact(analysis.images().len())
        .map_err(|_| Error::Allocation)?;
    samplers
        .try_reserve_exact(analysis.samplers().len())
        .map_err(|_| Error::Allocation)?;
    let description_bytes = images.capacity() * std::mem::size_of::<ImageDescription>()
        + samplers.capacity() * std::mem::size_of::<SamplerDescription>();
    budget.host(description_bytes - initial_description_bytes)?;
    budget.host(
        sampled.images.capacity() * std::mem::size_of::<crate::image_descriptor::Image2D>()
            - initial_image_bytes,
    )?;
    budget.host(
        sampled.samplers.capacity() * std::mem::size_of::<crate::image_descriptor::Sampler>()
            - initial_sampler_bytes,
    )?;
    for requirement in analysis.images() {
        if !requirement.user_sgpr.is_multiple_of(4)
            || requirement
                .user_sgpr
                .checked_add(8)
                .is_none_or(|end| end > users)
        {
            return Err(Error::Unsupported("sampled image user registers"));
        }
        budget.work(8)?;
        let mut words = [0; 8];
        for (index, word) in words.iter_mut().enumerate() {
            *word = sh(
                state,
                0x0c + u32::from(requirement.user_sgpr) + index as u32,
            )?;
        }
        let descriptor = crate::image_descriptor::decode_image(
            words,
            &crate::image_descriptor::Limits {
                max_dimension: budget.limits.max_dimension,
                max_levels: 16,
                max_bytes: budget.limits.max_host_bytes as u64,
            },
        )
        .map_err(|_| Error::Unsupported("sampled image descriptor"))?;
        if descriptor.counter_id.is_some() {
            return Err(Error::Unsupported("sampled image LOD feedback"));
        }
        if descriptor.min_lod_u4_8 != 0 {
            // The same S# may serve different images. Its clamp cannot replace
            // the image-specific clamp without an admitted shader/device mode.
            return Err(Error::Unsupported("sampled image minimum LOD"));
        }
        sampled.images.push(descriptor);
        images.push(ImageDescription {
            user_sgpr: requirement.user_sgpr,
        });
    }
    for requirement in analysis.samplers() {
        if !requirement.user_sgpr.is_multiple_of(4)
            || requirement
                .user_sgpr
                .checked_add(4)
                .is_none_or(|end| end > users)
        {
            return Err(Error::Unsupported("sampler user registers"));
        }
        budget.work(4)?;
        let mut words = [0; 4];
        for (index, word) in words.iter_mut().enumerate() {
            *word = sh(
                state,
                0x0c + u32::from(requirement.user_sgpr) + index as u32,
            )?;
        }
        sampled.samplers.push(
            crate::image_descriptor::decode_sampler(words)
                .map_err(|_| Error::Unsupported("sampled image sampler"))?,
        );
        samplers.push(SamplerDescription {
            user_sgpr: requirement.user_sgpr,
        });
    }
    // The same opaque IR and finite compiler budget continue into emission.
    // The caller supplies no new entry, interface or register policy here.
    budget.peak(compiler.max_bytes)?;
    let compiled = analysis
        .finish_with_images(description.as_slice(), &images, &samplers)
        .map_err(|_| Error::Compile)?;
    drop(images);
    drop(samplers);
    budget.release(description_bytes)?;
    budget.host(compiled.retained_host_bytes())?;
    validate_manifest(&compiled)?;
    let expected = if native {
        pentax_shader_compiler::VertexInstanceContract::NativeIndirectFirstInstanceZero
    } else {
        pentax_shader_compiler::VertexInstanceContract::Dynamic
    };
    if compiled.manifest().vertex_instance_contract() != expected {
        return Err(Error::Unsupported("native indirect compiler context"));
    }
    if compiled.manifest().buffers().len() != usize::from(source.is_some()) {
        return Err(Error::Unsupported("graphics buffer specialization"));
    }
    if compiled.manifest().images().len() != sampled.images.len()
        || compiled.manifest().samplers().len() != sampled.samplers.len()
    {
        return Err(Error::Unsupported("sampled resource specialization"));
    }
    Ok((compiled, source, sampled))
}

fn snapshot_resource(
    source: Option<BufferSource>,
    memory: &impl Memory,
    target: crate::compute::CommandRange,
    budget: &mut Budget<'_>,
) -> Result<Option<PreparedBuffer>, Error> {
    let Some(source) = source else {
        return Ok(None);
    };
    let range = source.range;
    if range.address < target.address + target.size && target.address < range.address + range.size {
        return Err(Error::Unsupported("render target overlaps buffer source"));
    }
    let length = usize::try_from(range.size).map_err(|_| Error::Limit("buffer bytes"))?;
    budget.host(length)?;
    budget.work(range.size)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Error::Allocation)?;
    budget.host(bytes.capacity() - length)?;
    bytes.resize(length, 0);
    memory
        .snapshot_buffer(range.address, &mut bytes, true, false)
        .map_err(Error::Memory)?;
    Ok(Some(PreparedBuffer { range, bytes }))
}

fn push_data(
    state: &GraphicsState,
    compiled: &CompiledShader,
    vertex: bool,
) -> Result<[u8; 128], Error> {
    let mut push = [0; 128];
    if let Some(layout) = compiled.manifest().push_data() {
        let first = if vertex { 0 } else { 16 };
        if layout.start_dword != first
            || layout.user_sgprs.len() > 16
            || layout.memory_offset_count != compiled.manifest().buffers().len() as u32
            || layout.memory_offset_count > 1
            || layout.memory_offset_dword != layout.user_sgprs.len() as u32
            || layout.user_sgprs.len() + layout.memory_offset_count.div_ceil(4) as usize > 16
        {
            return Err(Error::Unsupported("graphics push layout"));
        }
        // Each binding begins at the exact copied guest base. Packed prefix
        // bytes are explicitly zero and occupy the remaining stage window.
        if layout.memory_offset_count != 0 {
            let at = (first as usize + layout.memory_offset_dword as usize) * 4;
            push[at..at + 4].copy_from_slice(&0_u32.to_le_bytes());
        }
        for (index, register) in layout.user_sgprs.iter().enumerate() {
            if *register >= 16 {
                return Err(Error::Unsupported("graphics push SGPR"));
            }
            let value = sh(
                state,
                if vertex { 0x8c } else { 0x0c } + u32::from(*register),
            )?;
            let offset = (first as usize + index) * 4;
            push[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(push)
}

/// Compile this draw's real guest VS/PS and retain the actual GPU-readable and
/// GPU-writable target contents. Complete fixed-state admission precedes reads.
/// Unsupported shaders/resources never return a replacement or partial product.
pub fn prepare_draw(
    draw: &Draw,
    memory: &impl Memory,
    limits: &Limits,
) -> Result<PreparedDraw, Error> {
    prepare_draw_with_target(draw, memory, limits, |_, _, _| {
        Err(Error::Unsupported("tiled target decoder is not bound"))
    })
}

/// Prepare real shaders and a full source-layout LOAD snapshot. A tiled target
/// additionally requires the caller's bounded pure decoder; the native runtime
/// binds its existing source-pinned conversion, never a guest-memory callback.
pub fn prepare_draw_with_target(
    draw: &Draw,
    memory: &impl Memory,
    limits: &Limits,
    decode: impl FnOnce(&[u8], TargetGeometry, TargetDecodeLimits) -> Result<DecodedTarget, Error>,
) -> Result<PreparedDraw, Error> {
    let mut budget = Budget {
        limits,
        usage: Usage::default(),
    };
    let config = configuration(draw, &mut budget)?;
    let backend_pixels = config
        .geometry
        .tight_bytes
        .checked_mul(2)
        .ok_or(Error::Limit("pixel staging bytes"))?;
    let decoded_pixels = if config.geometry.layout == TargetLayout::LinearRgba8 {
        0
    } else {
        config.geometry.tight_bytes
    };
    budget.pixels(
        config
            .bytes
            .checked_add(decoded_pixels)
            .ok_or(Error::Limit("pixel staging bytes"))?,
        backend_pixels,
    )?;
    if limits.compiler_phase_bytes == 0 || limits.compiler_phase_work == 0 {
        return Err(Error::Limit("compiler budget"));
    }
    budget.work(
        limits
            .compiler_phase_work
            .checked_mul(6)
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
    let vertex_program = read_program(
        memory,
        config.vertex_address,
        ShaderStage::Vertex,
        &mut budget,
        &compiler,
    )?;
    let pixel_program = read_program(
        memory,
        config.pixel_address,
        ShaderStage::Pixel,
        &mut budget,
        &compiler,
    )?;
    let shader_ranges = [
        crate::compute::CommandRange {
            address: config.vertex_address,
            size: (vertex_program.source_words().len() as u64)
                .checked_mul(4)
                .ok_or(Error::Limit("shader range"))?,
        },
        crate::compute::CommandRange {
            address: config.pixel_address,
            size: (pixel_program.source_words().len() as u64)
                .checked_mul(4)
                .ok_or(Error::Limit("shader range"))?,
        },
    ];
    budget.work(2)?;
    let target_end = config
        .address
        .checked_add(config.bytes as u64)
        .ok_or(Error::Unsupported("render-target extent"))?;
    for range in shader_ranges {
        let end = range
            .address
            .checked_add(range.size)
            .ok_or(Error::Unsupported("shader extent"))?;
        if config.address < end && range.address < target_end {
            return Err(Error::Unsupported("render target overlaps shader source"));
        }
    }
    let vertex_work = budget
        .usage
        .vertices
        .checked_mul(vertex_program.instruction_count() as u64)
        .ok_or(Error::Limit("vertex work"))?;
    budget.work(vertex_work)?;
    let pixel_work = u64::from(config.width)
        .checked_mul(u64::from(config.height))
        .and_then(|n| n.checked_mul(u64::from(draw.vertex_count).div_ceil(3)))
        .and_then(|n| n.checked_mul(u64::from(draw.instance_count)))
        .and_then(|n| n.checked_mul(pixel_program.instruction_count() as u64))
        .and_then(|n| n.checked_mul(4))
        .ok_or(Error::Limit("fragment work"))?;
    budget.work(pixel_work)?;
    let mut vertex_common = CommonDescription::new(config.vertex_wave);
    vertex_common.user_sgpr_count = config.vertex_users;
    let vertex_profile = Profile::new(StageDescription::Vertex(VertexDescription {
        common: vertex_common,
        start_instance_sgpr: None,
    }))
    .map_err(|_| Error::Compile)?;
    let (pixel_description, pixel_profile_bytes) =
        config.pixel.profile(config.pixel_users, &mut budget)?;
    let pixel_profile =
        Profile::new(StageDescription::Pixel(pixel_description)).map_err(|_| Error::Compile)?;
    let (vertex, vertex_source, vertex_sampled) = compile_stage(
        &vertex_program,
        vertex_profile,
        draw,
        true,
        &compiler,
        &mut budget,
    )?;
    let (pixel, pixel_source, pixel_sampled) = compile_stage(
        &pixel_program,
        pixel_profile,
        draw,
        false,
        &compiler,
        &mut budget,
    )?;
    budget.work(256)?;
    validate_manifest(&vertex)?;
    validate_manifest(&pixel)?;
    let interfaces = validate_shader_interfaces(&vertex, &pixel)?;
    if (config.pixel.export_span < 32
        && interfaces.vertex_locations >> config.pixel.export_span != 0)
        || !config.pixel.matches(&pixel)
    {
        return Err(Error::Unsupported(
            "graphics parameter declaration mismatch",
        ));
    }
    let vertex_push = push_data(&draw.state, &vertex, true)?;
    let pixel_push = push_data(&draw.state, &pixel, false)?;
    let program_bytes = vertex_program
        .retained_host_bytes()
        .checked_add(pixel_program.retained_host_bytes())
        .ok_or(Error::Limit("compiler bytes"))?;
    // No decoded program, CFG/IR or compiler layout remains live when the
    // target is allocated. Keep only the actual compiled products charged.
    drop(vertex_program);
    drop(pixel_program);
    budget.release(program_bytes)?;
    budget.release(pixel_profile_bytes)?;
    let target_range = crate::compute::CommandRange {
        address: config.address,
        size: config.bytes as u64,
    };
    let vertex_buffer = snapshot_resource(vertex_source, memory, target_range, &mut budget)?;
    let pixel_buffer = snapshot_resource(pixel_source, memory, target_range, &mut budget)?;
    if !vertex_sampled.images.is_empty() || !vertex_sampled.samplers.is_empty() {
        return Err(Error::Unsupported("vertex sampled resource profile"));
    }
    let image_metadata_bytes =
        pixel_sampled.images.capacity() * std::mem::size_of::<crate::image_descriptor::Image2D>();
    let sampler_metadata_bytes =
        pixel_sampled.samplers.capacity() * std::mem::size_of::<crate::image_descriptor::Sampler>();
    let mut pixel_images = Vec::new();
    let mut pixel_samplers = Vec::new();
    let initial_prepared_image_bytes =
        pixel_sampled.images.len() * std::mem::size_of::<super::PreparedTexture>();
    let initial_prepared_sampler_bytes =
        pixel_sampled.samplers.len() * std::mem::size_of::<super::PreparedSampler>();
    budget.host(initial_prepared_image_bytes)?;
    budget.host(initial_prepared_sampler_bytes)?;
    pixel_images
        .try_reserve_exact(pixel_sampled.images.len())
        .map_err(|_| Error::Allocation)?;
    pixel_samplers
        .try_reserve_exact(pixel_sampled.samplers.len())
        .map_err(|_| Error::Allocation)?;
    budget.host(
        pixel_images.capacity() * std::mem::size_of::<super::PreparedTexture>()
            - initial_prepared_image_bytes,
    )?;
    budget.host(
        pixel_samplers.capacity() * std::mem::size_of::<super::PreparedSampler>()
            - initial_prepared_sampler_bytes,
    )?;
    for (descriptor, manifest) in pixel_sampled
        .images
        .into_iter()
        .zip(pixel.manifest().images())
    {
        pixel_images.push(super::textures::capture(
            descriptor,
            manifest.binding,
            memory,
            target_range,
            &mut budget,
        )?);
    }
    for (descriptor, manifest) in pixel_sampled
        .samplers
        .into_iter()
        .zip(pixel.manifest().samplers())
    {
        pixel_samplers.push(super::PreparedSampler {
            binding: manifest.binding,
            descriptor,
        });
    }
    budget.release(image_metadata_bytes)?;
    budget.release(sampler_metadata_bytes)?;
    let indices = if let Some(source) = draw.indices {
        let size = usize::try_from(draw.vertex_count)
            .ok()
            .and_then(|count| count.checked_mul(source.index_type.byte_width()))
            .filter(|size| *size != 0)
            .ok_or(Error::Limit("index bytes"))?;
        let end = source
            .address
            .checked_add(size as u64)
            .filter(|end| *end <= 1 << 47)
            .ok_or(Error::Unsupported("index extent"))?;
        if source.address == 0
            || !source
                .address
                .is_multiple_of(source.index_type.byte_width() as u64)
        {
            return Err(Error::Unsupported("index alignment"));
        }
        if source.address < target_end && config.address < end {
            return Err(Error::Unsupported("render target overlaps index source"));
        }
        budget.work(
            (size as u64)
                .checked_mul(2)
                .ok_or(Error::Limit("index work"))?,
        )?;
        budget.host(size)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| Error::Allocation)?;
        budget.host(bytes.capacity() - size)?;
        bytes.resize(size, 0);
        memory
            .snapshot_buffer(source.address, &mut bytes, true, false)
            .map_err(Error::Memory)?;
        Some(PreparedIndices {
            range: crate::compute::CommandRange {
                address: source.address,
                size: size as u64,
            },
            index_type: source.index_type,
            count: draw.vertex_count,
            bytes,
        })
    } else {
        None
    };
    budget.host(config.bytes)?;
    budget.work(config.bytes as u64)?;
    let mut input = Vec::new();
    input
        .try_reserve_exact(config.bytes)
        .map_err(|_| Error::Allocation)?;
    budget.host(input.capacity() - config.bytes)?;
    budget.pixels(
        input
            .capacity()
            .checked_add(decoded_pixels)
            .ok_or(Error::Limit("pixel staging bytes"))?,
        backend_pixels,
    )?;
    input.resize(config.bytes, 0);
    memory
        .snapshot_buffer(config.address, &mut input, true, true)
        .map_err(Error::Memory)?;
    let (input, target_snapshot) = if config.geometry.layout == TargetLayout::LinearRgba8 {
        (input, None)
    } else {
        let remaining = TargetDecodeLimits {
            max_host_bytes: limits
                .max_host_bytes
                .checked_sub(budget.usage.host_bytes)
                .map(|n| {
                    n.min(
                        limits
                            .max_pixel_bytes
                            .saturating_sub(input.capacity())
                            .saturating_sub(backend_pixels),
                    )
                })
                .filter(|n| *n >= config.geometry.tight_bytes)
                .ok_or(Error::Limit("target decode bytes"))?,
            max_work: limits
                .max_work
                .checked_sub(budget.usage.work)
                .ok_or(Error::Limit("target decode work"))?,
        };
        let decoded = decode(&input, config.geometry, remaining)?;
        if decoded.bytes.len() != config.geometry.tight_bytes {
            return Err(Error::Unsupported("target decode extent"));
        }
        budget.host(decoded.bytes.capacity())?;
        budget.pixels(
            input
                .capacity()
                .checked_add(decoded.bytes.capacity())
                .ok_or(Error::Limit("pixel staging bytes"))?,
            backend_pixels,
        )?;
        budget.work(decoded.work)?;
        (decoded.bytes, Some(input))
    };
    budget.usage.target_bytes = config.bytes;
    // The fixed256-step manifest/interface envelope above also covers this
    // constant-size product/context consistency check; it adds no allocation.
    let prepared = PreparedDraw {
        vertex,
        pixel,
        input,
        target_snapshot,
        target_geometry: config.geometry,
        vertex_push,
        pixel_push,
        target_address: config.address,
        width: config.width,
        height: config.height,
        vertex_count: draw.vertex_count,
        instance_count: draw.instance_count,
        native_offsets: draw.native_offsets,
        shader_ranges,
        indices,
        vertex_buffer,
        pixel_buffer,
        pixel_images,
        pixel_samplers,
        usage: budget.usage,
    };
    prepared.validate_native_instance_contract()?;
    Ok(prepared)
}
