// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit register/profile admission and owned resources.

use super::{
    Budget, DecodedTarget, Draw, Error, GraphicsState, Limits, Memory, PreparedDraw, RegisterSpace,
    TargetDecodeLimits, TargetGeometry, TargetLayout, Usage,
};
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{
    CommonDescription, CompiledShader, InputInterface, Limits as CompilerLimits, OutputInterface,
    PixelDescription, Profile, ShaderProgram, StageDescription, VertexDescription, WaveSize,
    compile,
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
    let vertices = u64::from(draw.vertex_count)
        .checked_mul(u64::from(draw.instance_count))
        .filter(|n| *n != 0 && *n <= budget.limits.max_vertices)
        .ok_or(Error::Limit("draw vertices"))?;
    budget.usage.vertices = vertices;
    // No DCC/CMASK/FMASK, layered views, depth, blending, logic operations,
    // clipping overrides, raster discard, extra viewports or sample shading.
    for register in [
        0x80, 0x1b1, 0x1b3, 0x1b4, 0x1b8, 0x1c4, 0x1e0, 0x200, 0x203, 0x205, 0x207, 0x293, 0x2f8,
        0x31b, 0x31d, 0x31e, 0x31f, 0x321, 0x325, 0x398, 0x3a0, 0x3a8,
    ] {
        expected(state, register, 0)?;
    }
    for (register, value) in [
        (0x8e, 0xf),
        (0x8f, 0xf),
        (0x1b6, 0x8000),
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

fn validate_manifest(compiled: &CompiledShader, vertex: bool) -> Result<(), Error> {
    let manifest = compiled.manifest();
    let features = manifest.required_features();
    if !manifest.buffers().is_empty()
        || manifest.buffer_binding().is_some()
        || manifest.shader_data_binding().is_some()
        || manifest.undefined_register_reads() != 0
        || manifest.required_subgroup_size().is_some()
        || features.subgroup_basic()
        || features.subgroup_ballot()
        || features.draw_parameters()
    {
        return Err(Error::Unsupported("graphics resource/subgroup manifest"));
    }
    if vertex {
        if manifest.outputs() != [OutputInterface::Position]
            || manifest.inputs().iter().any(|i| {
                !matches!(
                    i,
                    InputInterface::VertexIndex | InputInterface::InstanceIndex
                )
            })
        {
            return Err(Error::Unsupported("vertex interface manifest"));
        }
    } else if manifest.outputs()
        != [OutputInterface::Color {
            export: 0,
            location: 0,
            uint: false,
        }]
        || !manifest.inputs().is_empty()
    {
        return Err(Error::Unsupported("pixel interface manifest"));
    }
    Ok(())
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
            || layout.memory_offset_count != 0
        {
            return Err(Error::Unsupported("graphics push layout"));
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
    let mut pixel_common = CommonDescription::new(WaveSize::Wave32);
    pixel_common.user_sgpr_count = config.pixel_users;
    pixel_common.push_data_start_dword = 16;
    let pixel_profile = Profile::new(StageDescription::Pixel(PixelDescription::new(pixel_common)))
        .map_err(|_| Error::Compile)?;
    budget.peak(compiler.max_bytes)?;
    let vertex =
        compile(&vertex_program, &vertex_profile, &compiler).map_err(|_| Error::Compile)?;
    budget.host(vertex.retained_host_bytes())?;
    budget.peak(compiler.max_bytes)?;
    let pixel = compile(&pixel_program, &pixel_profile, &compiler).map_err(|_| Error::Compile)?;
    budget.host(pixel.retained_host_bytes())?;
    validate_manifest(&vertex, true)?;
    validate_manifest(&pixel, false)?;
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
    Ok(PreparedDraw {
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
        shader_ranges,
        usage: budget.usage,
    })
}
