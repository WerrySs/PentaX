// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX from public KytyPS5 shaderPixelParameter.cpp and
// ShaderGetStaticInputInfoPS; see NOTICE. No guest pointers or host effects.

use super::{Budget, Error, GraphicsState, RegisterSpace};
use pentax_shader_compiler::{
    CommonDescription, CompiledShader, InputInterface, OutputInterface, PixelDescription,
    PixelInput, WaveSize,
};

pub(super) struct PixelRegisters {
    inputs: [PixelInput; 32],
    count: usize,
    enables: u32,
    pub(super) export_span: u8,
    wave: WaveSize,
}

impl PixelRegisters {
    pub(super) fn read(state: &GraphicsState, budget: &mut Budget<'_>) -> Result<Self, Error> {
        budget.work(40)?;
        let get = |register| state.get(RegisterSpace::Context, register);
        let control = get(0x1b6)?;
        let exports = get(0x1b1)?;
        let enables = get(0x1b3)?;
        if control & !0x803f != 0 || control & 0x3f > 32 || exports & !0x3e != 0 {
            return Err(Error::Unsupported("graphics parameter declaration"));
        }
        // Only center perspective and compact position/front-face inputs are
        // consumed by the supported compiler. Never guess other input modes.
        if enables != get(0x1b4)? || enables & !0x1f02 != 0 || get(0x1b8)? != 0 {
            return Err(Error::Unsupported("pixel input register profile"));
        }
        let count = (control & 0x3f) as usize;
        let mut inputs = [PixelInput {
            location: 0,
            flat: false,
        }; 32];
        let mut flat_locations = 0_u32;
        let mut smooth_locations = 0_u32;
        for (index, input) in inputs.iter_mut().enumerate().take(count) {
            let value = get(0x191 + index as u32)?;
            // AGC-produced custom non-F16 words set bit5. F16/default fields
            // are excluded too; none is reinterpreted as ordinary flat input.
            if value & !0x41f != 0 {
                return Err(Error::Unsupported("pixel parameter control"));
            }
            *input = PixelInput {
                location: (value & 31) as u8,
                flat: value & 0x400 != 0,
            };
            let bit = 1_u32 << input.location;
            if input.flat {
                flat_locations |= bit;
            } else {
                smooth_locations |= bit;
            }
        }
        if flat_locations & smooth_locations != 0 {
            return Err(Error::Unsupported("conflicting pixel parameter aliases"));
        }
        Ok(Self {
            inputs,
            count,
            enables,
            export_span: (1 + (exports >> 1)) as u8,
            wave: if control & 0x8000 != 0 {
                WaveSize::Wave32
            } else {
                WaveSize::Wave64
            },
        })
    }

    pub(super) fn profile(
        &self,
        users: u8,
        budget: &mut Budget<'_>,
    ) -> Result<(PixelDescription, usize), Error> {
        let bytes = self.count * std::mem::size_of::<PixelInput>();
        budget.host(bytes)?;
        let mut common = CommonDescription::new(self.wave);
        common.user_sgpr_count = users;
        common.push_data_start_dword = 16;
        let mut pixel = PixelDescription::new(common);
        pixel
            .inputs
            .try_reserve_exact(self.count)
            .map_err(|_| Error::Allocation)?;
        let actual = pixel
            .inputs
            .capacity()
            .checked_mul(std::mem::size_of::<PixelInput>())
            .ok_or(Error::Limit("pixel input bytes"))?;
        budget.host(actual - bytes)?;
        pixel.inputs.extend_from_slice(&self.inputs[..self.count]);
        pixel.perspective_center_vgpr = (self.enables & 2 != 0).then_some(0);
        pixel.system_input_base = if self.enables & 2 != 0 { 2 } else { 0 };
        pixel.position = std::array::from_fn(|i| self.enables & (1 << (8 + i)) != 0);
        pixel.front_face = self.enables & (1 << 12) != 0;
        Ok((pixel, actual))
    }

    pub(super) fn matches(&self, pixel: &CompiledShader) -> bool {
        pixel.manifest().inputs().iter().all(|input| match *input {
            InputInterface::Parameter {
                input,
                location,
                flat,
            } => self
                .inputs
                .get(input as usize)
                .filter(|_| (input as usize) < self.count)
                .is_some_and(|expected| {
                    u32::from(expected.location) == location && expected.flat == flat
                }),
            InputInterface::FragCoord => self.enables & 0xf00 != 0,
            InputInterface::FrontFacing => self.enables & 0x1000 != 0,
            _ => false,
        })
    }
}

/// Allocation-free facts from actual compiler interfaces, not device support,
/// guest ownership, resource permissions or execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderInterfaces {
    pub vertex_locations: u32,
    pub pixel_locations: u32,
    pub vertex_components: u32,
    pub pixel_components: u32,
}

/// Validate every consumed cross-stage PARAM. Same-location pixel aliases
/// require matching interpolation; sparse and unused vertex exports are valid.
pub fn validate_shader_interfaces(
    vertex: &CompiledShader,
    pixel: &CompiledShader,
) -> Result<ShaderInterfaces, Error> {
    let mut vertex_locations = 0_u32;
    let mut position = false;
    for output in vertex.manifest().outputs() {
        match *output {
            OutputInterface::Position if !position => position = true,
            OutputInterface::Parameter { index, location }
                if index == location
                    && location < 32
                    && vertex_locations & (1 << location) == 0 =>
            {
                vertex_locations |= 1 << location
            }
            _ => return Err(Error::Unsupported("vertex interface manifest")),
        }
    }
    if !position
        || vertex.manifest().inputs().iter().any(|input| {
            !matches!(
                input,
                InputInterface::VertexIndex | InputInterface::InstanceIndex
            )
        })
    {
        return Err(Error::Unsupported("vertex interface manifest"));
    }
    if pixel.manifest().outputs()
        != [OutputInterface::Color {
            export: 0,
            location: 0,
            uint: false,
        }]
    {
        return Err(Error::Unsupported("pixel interface manifest"));
    }
    let mut pixel_locations = 0_u32;
    let mut flat_locations = 0_u32;
    let mut input_indices = 0_u32;
    let mut system = 0_u32;
    for input in pixel.manifest().inputs() {
        match *input {
            InputInterface::Parameter {
                input,
                location,
                flat,
            } if input < 32 && location < 32 => {
                let bit = 1_u32 << location;
                if input_indices & (1 << input) != 0
                    || vertex_locations & bit == 0
                    || pixel_locations & bit != 0 && (flat_locations & bit != 0) != flat
                {
                    return Err(Error::Unsupported("cross-stage parameter linkage"));
                }
                input_indices |= 1 << input;
                pixel_locations |= bit;
                if flat {
                    flat_locations |= bit;
                }
            }
            InputInterface::FragCoord if system & 1 == 0 => system |= 1,
            InputInterface::FrontFacing if system & 2 == 0 => system |= 2,
            _ => return Err(Error::Unsupported("pixel interface manifest")),
        }
    }
    Ok(ShaderInterfaces {
        vertex_locations,
        pixel_locations,
        vertex_components: 4 + 4 * vertex_locations.count_ones(),
        pixel_components: 4 * pixel_locations.count_ones()
            + if system & 1 != 0 { 4 } else { 0 }
            + if system & 2 != 0 { 1 } else { 0 },
    })
}
