// SPDX-License-Identifier: GPL-2.0-only
//! Pure checked T#/S# metadata for sampled linear RGBA8 UNORM 2D images.
//!
//! Descriptor bitfields follow public KytyPS5 shaderBindings.h. Linear storage
//! follows TileGetTextureSize/SetLinearMipChainLayout: 256-byte rows and reversed
//! mip placement. No decode grants memory, queue, image, sampler or GPU authority.
//! Views requiring physical mip remapping and all unsupported controls refuse.
//! Public source: https://github.com/Jetsku/KytyPS5/tree/b719d67ce6bcfd21d402bd415f3e4f68e4793534/src/graphics

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Unsupported(&'static str),
    Limit(&'static str),
    AddressOverflow,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_dimension: u32,
    pub max_levels: u8,
    pub max_bytes: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Component {
    Zero,
    One,
    Red,
    Green,
    Blue,
    Alpha,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MipLevel {
    pub offset: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub storage_height: u32,
    pub row_pitch_bytes: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Image2D {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub base_level: u8,
    pub last_level: u8,
    /// Absolute T# clamp, in U4.8 units; the view base is not subtracted here.
    pub min_lod_u4_8: u16,
    pub swizzle: [Component; 4],
    pub counter_id: Option<u8>,
    /// Retained source scheduling hint; this profile admits only encodings0/7.
    pub perf_mod: u8,
    pub mips: [MipLevel; 16],
    pub level_count: u8,
    pub total_bytes: u64,
}
fn component(value: u32) -> Result<Component, Error> {
    match value {
        0 => Ok(Component::Zero),
        1 => Ok(Component::One),
        4 => Ok(Component::Red),
        5 => Ok(Component::Green),
        6 => Ok(Component::Blue),
        7 => Ok(Component::Alpha),
        _ => Err(Error::Unsupported("image component selection")),
    }
}
/// Decode metadata only; the caller must admit and read the entire current
/// owned source span at the actual ordered draw/dispatch turn.
pub fn decode_image(words: [u32; 8], limits: &Limits) -> Result<Image2D, Error> {
    if words[1] & 0x2000_0000 != 0 || words[2] & 0xf000_3000 != 0 {
        return Err(Error::Unsupported("image reserved fields"));
    }
    if (words[1] >> 20) & 0x1ff != 56 {
        return Err(Error::Unsupported("image RGBA8 UNORM format"));
    }
    if words[3] >> 28 != 9 {
        return Err(Error::Unsupported("sampled nonarray 2D image"));
    }
    if (words[3] >> 20) & 0x1f != 0 {
        return Err(Error::Unsupported("linear image layout"));
    }
    if words[3] & 0x0e00_0000 != 0 || words[4] != 0 {
        return Err(Error::Unsupported(
            "image block swizzle or array/depth fields",
        ));
    }
    // Preserve the recognized performance hint rather than infer numerical or
    // residency semantics. This is a bounded profile, not a hardware validity mask.
    let perf_mod = ((words[5] >> 20) & 7) as u8;
    if words[5] & !0x0270_00f0 != 0 || !matches!(perf_mod, 0 | 7) {
        return Err(Error::Unsupported(
            "image warning, sparse or sampling controls",
        ));
    }
    if words[6] & !0xff != 0 || words[7] != 0 {
        return Err(Error::Unsupported("image compression or metadata fields"));
    }
    let address = (u64::from(words[0]) | (u64::from(words[1] & 0xff) << 32)) << 8;
    if address == 0 {
        return Err(Error::Unsupported("null sampled image"));
    }
    let width = ((words[1] >> 30) | ((words[2] & 0xfff) << 2)) + 1;
    let height = ((words[2] >> 14) & 0x3fff) + 1;
    if width > limits.max_dimension || height > limits.max_dimension {
        return Err(Error::Limit("image dimensions"));
    }
    let base_level = ((words[3] >> 12) & 15) as u8;
    let last_level = ((words[3] >> 16) & 15) as u8;
    let max_mip = ((words[5] >> 4) & 15) as u8;
    if base_level > last_level || last_level > max_mip {
        return Err(Error::Unsupported("image physical mip view remapping"));
    }
    let level_count = max_mip + 1;
    if level_count > limits.max_levels {
        return Err(Error::Limit("image mip levels"));
    }
    let min_lod_u4_8 = ((words[1] >> 8) & 0xfff) as u16;
    if u32::from(min_lod_u4_8) > u32::from(last_level) * 256 {
        return Err(Error::Unsupported("image minimum LOD beyond view"));
    }
    let swizzle = [
        component(words[3] & 7)?,
        component((words[3] >> 3) & 7)?,
        component((words[3] >> 6) & 7)?,
        component((words[3] >> 9) & 7)?,
    ];
    let mut mips = [MipLevel::default(); 16];
    let mut total_bytes = 0_u64;
    for level in (0..level_count).rev() {
        let divisor = 1_u32 << level;
        let row_pitch_bytes = width.div_ceil(divisor).max(1).div_ceil(64) * 256;
        let storage_height = height.div_ceil(divisor).max(1);
        let size = u64::from(row_pitch_bytes)
            .checked_mul(u64::from(storage_height))
            .ok_or(Error::AddressOverflow)?;
        mips[usize::from(level)] = MipLevel {
            offset: total_bytes,
            size,
            width: (width >> level).max(1),
            height: (height >> level).max(1),
            storage_height,
            row_pitch_bytes,
        };
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or(Error::AddressOverflow)?;
        if total_bytes > limits.max_bytes {
            return Err(Error::Limit("image storage bytes"));
        }
    }
    address
        .checked_add(total_bytes)
        .ok_or(Error::AddressOverflow)?;
    Ok(Image2D {
        address,
        width,
        height,
        base_level,
        last_level,
        min_lod_u4_8,
        swizzle,
        counter_id: if words[5] & (1 << 25) != 0 {
            Some(words[6] as u8)
        } else {
            None
        },
        perf_mod,
        mips,
        level_count,
        total_bytes,
    })
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Filter {
    Nearest,
    Linear,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MipFilter {
    None,
    Nearest,
    Linear,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressMode {
    Repeat,
    MirroredRepeat,
    ClampToEdge,
    ClampToBorder,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BorderColor {
    TransparentBlack,
    OpaqueBlack,
    OpaqueWhite,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sampler {
    pub address_modes: [AddressMode; 3],
    pub mag_filter: Filter,
    pub min_filter: Filter,
    pub mip_filter: MipFilter,
    /// Effective clamps; mip filtering disabled forces both to zero.
    pub min_lod_u4_8: u16,
    pub max_lod_u4_8: u16,
    pub lod_bias_s6_8: i16,
    pub border: BorderColor,
}
fn address_mode(value: u32) -> Result<AddressMode, Error> {
    match value {
        0 => Ok(AddressMode::Repeat),
        1 => Ok(AddressMode::MirroredRepeat),
        2 => Ok(AddressMode::ClampToEdge),
        6 => Ok(AddressMode::ClampToBorder),
        _ => Err(Error::Unsupported("sampler address mode")),
    }
}
fn filter(value: u32) -> Result<Filter, Error> {
    match value {
        0 => Ok(Filter::Nearest),
        1 => Ok(Filter::Linear),
        _ => Err(Error::Unsupported("sampler anisotropic filter")),
    }
}
/// Normalized, noncomparison sampler. Backend feature/device admission still
/// owns all Vulkan limits and must not silently clamp an unsupported bias.
pub fn decode_sampler(words: [u32; 4]) -> Result<Sampler, Error> {
    if words[0] & !0x1ff != 0
        || words[1] & 0xff00_0000 != 0
        || words[2] & !0x0cf0_3fff != 0
        || words[3] & 0x3fff_ffff != 0
    {
        return Err(Error::Unsupported(
            "sampler comparison, unnormalized, reduction or extra controls",
        ));
    }
    let mip_filter = match (words[2] >> 26) & 3 {
        0 => MipFilter::None,
        1 => MipFilter::Nearest,
        2 => MipFilter::Linear,
        _ => return Err(Error::Unsupported("sampler mip filter")),
    };
    let (min_lod_u4_8, max_lod_u4_8) = if mip_filter == MipFilter::None {
        (0, 0)
    } else {
        ((words[1] & 0xfff) as u16, ((words[1] >> 12) & 0xfff) as u16)
    };
    if min_lod_u4_8 > max_lod_u4_8 {
        return Err(Error::Unsupported("sampler inverted LOD clamp"));
    }
    let raw_bias = (words[2] & 0x3fff) as u16;
    let lod_bias_s6_8 = ((raw_bias ^ 0x2000) as i16) - 0x2000;
    Ok(Sampler {
        address_modes: [
            address_mode(words[0] & 7)?,
            address_mode((words[0] >> 3) & 7)?,
            address_mode((words[0] >> 6) & 7)?,
        ],
        mag_filter: filter((words[2] >> 20) & 3)?,
        min_filter: filter((words[2] >> 22) & 3)?,
        mip_filter,
        min_lod_u4_8,
        max_lod_u4_8,
        lod_bias_s6_8,
        border: match words[3] >> 30 {
            0 => BorderColor::TransparentBlack,
            1 => BorderColor::OpaqueBlack,
            2 => BorderColor::OpaqueWhite,
            _ => return Err(Error::Unsupported("sampler border color table")),
        },
    })
}
#[cfg(test)]
#[path = "image_descriptor/tests.rs"]
mod tests;
