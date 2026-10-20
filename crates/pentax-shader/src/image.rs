// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, ImageOps.cpp:
// MIMG decoding (sample/gather/atomic/misc maps, NSA, BVH address counts and
// field-dependent restrictions) and ImageAddressComponentLayout/DwordCount.
// PentaX modifications: the full 2 + NSA length is proven before any body
// read; the source's last SetUnsupported reason is returned as a typed error
// after operand aborts, which still take precedence as in the source.

use crate::decode::{Input, record, scalar_source, unsupported, unsupported_opcode};
use crate::model::{Image, MAX_IMAGE_NSA_ADDRESSES, MemoryAccess, Record, memory_flag as mf};
use crate::opcode;
use crate::{
    DecodeError, Family, ImageAddressComponent, ImageDimension, ImageSampleFlags as F, Opcode as O,
    Operand, UnsupportedReason as R,
};

fn bit(value: u32, shift: u32) -> bool {
    (value >> shift) & 1 != 0
}

fn coord_components(dimension: ImageDimension) -> u8 {
    match dimension {
        ImageDimension::Dim1D => 1,
        ImageDimension::Dim1DArray => 2,
        ImageDimension::Dim2DMsaa | ImageDimension::Dim3D | ImageDimension::Dim2DArray => 3,
        ImageDimension::Dim2DMsaaArray => 4,
        ImageDimension::Dim2D => 2,
    }
}

fn gradient_components(dimension: ImageDimension) -> u8 {
    match dimension {
        ImageDimension::Dim1D | ImageDimension::Dim1DArray => 1,
        ImageDimension::Dim3D => 3,
        _ => 2,
    }
}

fn sample_address_components(flags: F, dimension: ImageDimension) -> u8 {
    let mut components = coord_components(dimension);
    for flag in [F::OFFSET, F::COMPARE, F::BIAS, F::LOD, F::LOD_CLAMP] {
        components += u8::from(flags.contains(flag));
    }
    if flags.contains(F::DERIVATIVE) {
        components += gradient_components(dimension) * 2;
    }
    components
}

fn width(flags: F, index: u32) -> u32 {
    if !flags.contains(F::A16) {
        return 32;
    }
    let mut cursor = 0;
    if flags.contains(F::OFFSET) {
        if index == cursor {
            return 32;
        }
        cursor += 1;
    }
    if flags.contains(F::BIAS) {
        if index == cursor {
            return 16;
        }
        cursor += 1;
    }
    if flags.contains(F::COMPARE) && index == cursor {
        return 32;
    }
    16
}

/// The source `ImageAddressComponentLayout`: packed position of address
/// component `component` for these sample flags, aligning 32-bit components.
/// It is the source's generic sample/gather layout and has no BVH meaning.
pub fn image_address_component_layout(flags: F, component: u8) -> ImageAddressComponent {
    let component = u32::from(component);
    let mut offset = 0u32;
    for index in 0..component {
        let bits = width(flags, index);
        if bits == 32 {
            offset = (offset + 31) & !31;
        }
        offset += bits;
    }
    let bits = width(flags, component);
    if bits == 32 {
        offset = (offset + 31) & !31;
    }
    ImageAddressComponent {
        bit_offset: offset,
        bit_width: bits,
    }
}

/// The source `ImageAddressDwordCount`.
pub fn image_address_dword_count(flags: F, components: u8) -> u32 {
    if components == 0 {
        return 0;
    }
    let last = image_address_component_layout(flags, components - 1);
    (last.bit_offset + last.bit_width).div_ceil(32)
}

pub(crate) fn decode_mimg(
    input: &mut Input<'_>,
    w0: u32,
    at: usize,
) -> Result<Record, DecodeError> {
    let family = Family::Mimg;
    let id = (((w0 >> 18) & 0x7f) | ((w0 & 1) << 7)) as u16;
    let nsa_dwords = ((w0 >> 1) & 3) as u8;
    let mut inst = record(at, family, id, O::ImageLoad);
    // Word 1 and every NSA word are read by the source; prove them all first.
    input.require(&mut inst, 2 + usize::from(nsa_dwords))?;
    let w1 = input.word(&inst, 1)?;
    let dimension_bits = ((w0 >> 3) & 7) as u8;
    let dimension = ImageDimension::from_bits(dimension_bits);
    let r128 = bit(w0, 15);
    let a16 = bit(w1, 30);
    let d16 = bit(w1, 31);
    let sample = opcode::mimg_sample(id);
    let gather = opcode::mimg_gather(id);
    let atomic = opcode::mimg_atomic(id);
    let op = if sample.is_some() {
        Some(O::ImageSample)
    } else if let Some((op, _)) = gather {
        Some(op)
    } else if let Some((op, _)) = atomic {
        Some(op)
    } else {
        match id {
            0x00 => Some(O::ImageLoad),
            0x01 => Some(O::ImageLoadMip),
            0x08 => Some(O::ImageStore),
            0x09 => Some(O::ImageStoreMip),
            0x0e => Some(O::ImageGetResinfo),
            0x60 => Some(O::ImageGetLod),
            0xe6 => Some(O::ImageBvhIntersectRay),
            0xe7 => Some(O::ImageBvh64IntersectRay),
            _ => None,
        }
    };
    let dmask = ((w0 >> 8) & 0xf) as u8;
    let data_components = if gather.is_some() {
        4
    } else {
        (dmask.count_ones() as u8).max(1)
    };
    let mut access = MemoryAccess::new(
        if d16 {
            data_components.div_ceil(2)
        } else {
            data_components
        },
        if d16 { 16 } else { 32 },
    );
    access.set(mf::GLC, bit(w0, 13));
    access.set(mf::SLC, bit(w0, 25));
    let (tfe, lwe) = (bit(w0, 16), bit(w0, 17));
    access.set(mf::TFE, tfe);
    access.set(mf::LWE, lwe);
    let mut nsa_addresses = [0u8; MAX_IMAGE_NSA_ADDRESSES];
    for (index, address) in nsa_addresses
        .iter_mut()
        .take(usize::from(nsa_dwords) * 4)
        .enumerate()
    {
        let word = input.word(&inst, 2 + index / 4)?;
        *address = (word >> ((index % 4) * 8)) as u8;
    }
    let table_flags = sample
        .map(|(_, flags)| flags)
        .or(gather.map(|(_, flags)| flags));
    let mut address_components = if let Some(table_flags) = table_flags {
        sample_address_components(table_flags, dimension)
    } else if atomic.is_some() {
        coord_components(dimension)
    } else {
        match id {
            0x0e => 1,
            0x01 | 0x09 => coord_components(dimension) + 1,
            0x00 | 0x08 | 0x60 => coord_components(dimension),
            _ => 0,
        }
    };
    let mut flags = table_flags.unwrap_or(F::NONE);
    if a16 {
        flags = flags.union(F::A16);
    }
    let bvh = matches!(
        op,
        Some(O::ImageBvhIntersectRay | O::ImageBvh64IntersectRay)
    );
    if bvh {
        // Node pointer (two dwords for BVH64), extent and origin, then the six
        // f32 direction dwords or three packed A16 dwords.
        address_components = if op == Some(O::ImageBvh64IntersectRay) {
            2
        } else {
            1
        } + 4
            + if a16 { 3 } else { 6 };
    }
    // Later source SetUnsupported calls overwrite the reason; keep the last.
    let reason = |reason| Some(unsupported(at, family, id, reason));
    let mut pending = if op.is_none() {
        Some(unsupported_opcode(at, family, id))
    } else if bvh && dmask != 0xf {
        reason(R::BvhDmask)
    } else if bvh && !r128 {
        reason(R::BvhR128)
    } else if bvh && (tfe || lwe) {
        reason(R::BvhTfeLwe)
    } else if bvh && nsa_dwords != 0 && 1 + nsa_dwords * 4 < address_components {
        reason(R::BvhNsaAddresses)
    } else {
        None
    };
    if gather.is_some() && dmask.count_ones() != 1 {
        pending = reason(R::MimgGatherDmask);
    }
    if let Some((atomic_op, supports_64)) = atomic {
        let compare_swap = atomic_op == O::ImageAtomicCmpswap;
        let (mask32, mask64) = if compare_swap { (0x3, 0xf) } else { (0x1, 0x3) };
        if dmask != mask32 && dmask != mask64 {
            pending = reason(R::MimgAtomicDmask);
        } else if dmask == mask64 {
            access.data_bits = 64;
            if !supports_64 {
                pending = reason(R::MimgAtomic64);
            }
        }
    }
    let supports_d16 =
        sample.is_some() || gather.is_some() || matches!(id, 0x00 | 0x01 | 0x08 | 0x09);
    if d16 && !supports_d16 {
        pending = reason(R::MimgD16);
    }
    inst.memory = Some(access);
    inst.image = Some(Image {
        dmask,
        data_components,
        sample_flags: flags,
        dimension_bits,
        address_components,
        nsa_dwords,
        nsa_addresses,
        r128,
    });
    inst.dst = Operand::vgpr(((w1 >> 8) & 0xff) as u16);
    inst.src[0] = Operand::vgpr((w1 & 0xff) as u16);
    inst.src[1] = scalar_source((((w1 >> 16) & 0x1f) * 4) as u16, at)?;
    inst.src[2] = scalar_source((((w1 >> 21) & 0x1f) * 4) as u16, at)?;
    inst.src_count = 3;
    if let Some(error) = pending {
        return Err(error);
    }
    // `op` is present whenever no error is pending.
    inst.opcode = op.ok_or(unsupported_opcode(at, family, id))?;
    Ok(inst)
}
