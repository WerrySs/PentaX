// SPDX-License-Identifier: GPL-2.0-only
// New synthetic SMEM/MUBUF/MTBUF/FLAT/DS/MIMG tests for the PentaX Rust port.
// Every word is composed here from field values; no guest/game fixtures.

use super::vector::{bad_opcode, reason, truncated};
use super::*;
use UnsupportedReason as R;

pub(super) const fn smem(op: u32, sdst: u32, sbase: u32, glc: u32) -> u32 {
    0xf400_0000 | (op << 18) | (glc << 16) | (sdst << 6) | sbase
}
pub(super) const fn smem1(soffset: u32, offset: u32) -> u32 {
    (soffset << 25) | offset
}
pub(super) const fn mubuf(op: u32, offset: u32, offen: u32, idxen: u32, glc: u32, dlc: u32) -> u32 {
    0xe000_0000
        | ((op & 0x7f) << 18)
        | ((op >> 7) << 25)
        | (dlc << 15)
        | (glc << 14)
        | (idxen << 13)
        | (offen << 12)
        | offset
}
pub(super) const fn buf1(
    vaddr: u32,
    vdata: u32,
    srsrc: u32,
    slc: u32,
    tfe: u32,
    soffset: u32,
) -> u32 {
    (soffset << 24) | (tfe << 23) | (slc << 22) | (srsrc << 16) | (vdata << 8) | vaddr
}
const fn mtbuf(op: u32, dfmt: u32, nfmt: u32) -> u32 {
    0xe800_0000 | (nfmt << 23) | (dfmt << 19) | ((op & 7) << 16)
}
pub(super) const fn flat(op: u32, seg: u32, lds: u32, offset: u32) -> u32 {
    0xdc00_0000 | (op << 18) | (seg << 14) | (lds << 13) | offset
}
pub(super) const fn flat1(addr: u32, data: u32, saddr: u32, vdst: u32) -> u32 {
    (vdst << 24) | (saddr << 16) | (data << 8) | addr
}
pub(super) const fn ds(op: u32, gds: u32, offset0: u32, offset1: u32) -> u32 {
    0xd800_0000 | (op << 18) | (gds << 17) | (offset1 << 8) | offset0
}
pub(super) const fn ds1(addr: u32, data0: u32, data1: u32, vdst: u32) -> u32 {
    (vdst << 24) | (data1 << 16) | (data0 << 8) | addr
}

#[derive(Clone, Copy)]
pub(super) struct Mimg {
    pub op: u32,
    pub nsa: u32,
    pub dim: u32,
    pub dmask: u32,
    pub glc: u32,
    pub r128: u32,
    pub tfe: u32,
    pub lwe: u32,
    pub slc: u32,
    pub vaddr: u32,
    pub vdata: u32,
    pub srsrc: u32,
    pub ssamp: u32,
    pub a16: u32,
    pub d16: u32,
}

impl Mimg {
    pub const fn new(op: u32, dmask: u32) -> Self {
        Self {
            op,
            nsa: 0,
            dim: 1,
            dmask,
            glc: 0,
            r128: 0,
            tfe: 0,
            lwe: 0,
            slc: 0,
            vaddr: 0,
            vdata: 0,
            srsrc: 0,
            ssamp: 0,
            a16: 0,
            d16: 0,
        }
    }
    pub fn words(self) -> Vec<u32> {
        let w0 = 0xf000_0000
            | ((self.op & 0x7f) << 18)
            | (self.slc << 25)
            | (self.lwe << 17)
            | (self.tfe << 16)
            | (self.r128 << 15)
            | (self.glc << 13)
            | (self.dmask << 8)
            | (self.dim << 3)
            | (self.nsa << 1)
            | (self.op >> 7);
        let w1 = (self.d16 << 31)
            | (self.a16 << 30)
            | (self.ssamp << 21)
            | (self.srsrc << 16)
            | (self.vdata << 8)
            | self.vaddr;
        let mut words = vec![w0, w1];
        // NSA addresses 10, 11, 12, ... in byte order.
        for dword in 0..self.nsa {
            let base = 10 + dword * 4;
            words.push(base | ((base + 1) << 8) | ((base + 2) << 16) | ((base + 3) << 24));
        }
        words
    }
}

fn err(words: &[u32]) -> DecodeError {
    decode(words, ShaderStage::Compute, &Limits::default()).unwrap_err()
}

fn count(words: impl Fn(u32) -> Vec<u32>, range: u32) -> usize {
    (0..range)
        .filter(|&id| {
            !matches!(
                decode(&words(id), ShaderStage::Unknown, &Limits::default()),
                Err(DecodeError::UnsupportedOpcode { .. })
            )
        })
        .count()
}

#[test]
fn memory_and_image_opcode_maps_match_the_source_table_sizes() {
    assert_eq!(count(|id| vec![smem(id, 0, 0, 0), 0], 256), 13);
    assert_eq!(count(|id| vec![mubuf(id, 0, 0, 0, 0, 0), 0], 256), 52);
    assert_eq!(count(|id| vec![mtbuf(id, 0, 0), (id >> 3) << 21], 16), 8);
    assert_eq!(count(|id| vec![flat(id, 0, 0, 0), 0], 128), 15);
    // DS restrictions may reject mapped opcodes with other reasons; only
    // unmapped encodings return UnsupportedOpcode.
    assert_eq!(count(|id| vec![ds(id, 0, 0, 0), 0], 256), 62);
    // 56 sample, 8 gather, 15 atomic and 8 other MIMG encodings.
    assert_eq!(count(|id| Mimg::new(id, 1).words(), 256), 87);
}

#[test]
fn smem_fields_operands_and_continued_decoding_after_unmapped_opcodes() {
    let span = decoded(&[
        smem(0x02, 8, 3, 1),
        smem1(124, 0x10_0000),
        smem(0x25, 4, 0, 0),
        0,
        smem(0x20, 4, 1, 0),
        0,
    ]);
    let load = span.instruction(0).unwrap();
    assert_eq!(load.opcode(), Opcode::SLoadDwordx4);
    let memory = load.memory().unwrap();
    assert_eq!(
        (memory.offset(), memory.signed_offset()),
        (0xfff0_0000, -0x10_0000)
    );
    assert!(memory.glc());
    assert_eq!(memory.data_dwords(), 4);
    assert_eq!(
        (load.destination().register(), load.sources()[0].register()),
        (8, 6)
    );
    assert_eq!(load.sources()[1].kind(), OperandKind::M0);
    let realtime = span.instruction(1).unwrap();
    assert!(realtime.sources().is_empty());
    assert_eq!(realtime.destination().register(), 4);
    let invalidate = span.instruction(2).unwrap();
    assert_eq!(invalidate.destination().kind(), OperandKind::Null);
    assert_eq!(invalidate.memory().unwrap().data_dwords(), 0);
    // SBASE pair 54 is selector 108, which aborts in the source.
    assert_eq!(
        err(&[smem(0x00, 0, 54, 0), 0]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 108,
            destination: false
        }
    );
    // The source keeps decoding an unmapped SMEM opcode: an operand abort wins.
    assert_eq!(
        err(&[smem(0x05, 0, 0, 0), smem1(110, 0)]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 110,
            destination: false
        }
    );
    assert_eq!(
        err(&[smem(0x05, 0, 0, 0), 0]),
        bad_opcode(Family::Smem, 0x05)
    );
    // Even a cache invalidation decodes SDST first.
    assert_eq!(
        err(&[smem(0x20, 110, 0, 0), 0]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 110,
            destination: true
        }
    );
}

#[test]
fn mubuf_and_mtbuf_fields_resources_and_unresolved_literal_soffset() {
    let span = decoded(&[mubuf(0x80, 0xfff, 1, 1, 1, 1), buf1(3, 4, 2, 1, 1, 255)]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.opcode(), Opcode::BufferLoadFormatD16X);
    assert_eq!(inst.opcode_id(), 0x80);
    let m = inst.memory().unwrap();
    assert_eq!((m.offset(), m.data_bits(), m.data_dwords()), (0xfff, 16, 1));
    assert!(m.offen() && m.idxen() && m.glc() && m.dlc() && m.slc() && m.tfe() && m.formatted());
    assert!(!m.typed());
    assert_eq!(inst.destination().sdwa_select(), SdwaSelect::Word0);
    assert_eq!(inst.sources()[1].register(), 8);
    // SOFFSET 255 keeps a literal kind with value 0; no literal word is read.
    assert_eq!(inst.raw_words().len(), 2);
    assert_eq!(
        (inst.sources()[2].kind(), inst.sources()[2].value()),
        (OperandKind::LiteralConstant, 0)
    );
    // Signed byte load and invalidations that ignore all operand fields.
    let span = decoded(&[
        mubuf(0x09, 0, 0, 0, 0, 0),
        buf1(0, 0, 0, 0, 0, 128),
        mubuf(0x71, 0, 0, 0, 0, 0),
        buf1(0, 0, 27, 0, 0, 250),
    ]);
    assert!(span.instruction(0).unwrap().memory().unwrap().data_signed());
    let inv = span.instruction(1).unwrap();
    assert_eq!(inv.destination().kind(), OperandKind::Null);
    assert!(inv.sources().is_empty());
    // SRSRC 27 is selector 108; the unmapped opcode still reaches it.
    assert_eq!(
        err(&[mubuf(0x10, 0, 0, 0, 0, 0), buf1(0, 0, 27, 0, 0, 0)]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 108,
            destination: false
        }
    );
    assert_eq!(
        err(&[mubuf(0x10, 0, 0, 0, 0, 0), 0]),
        bad_opcode(Family::Mubuf, 0x10)
    );
    // MTBUF: opcode bit 3 from word 1, DFMT/NFMT retained.
    let span = decoded(&[mtbuf(0x07, 0xa, 5), buf1(1, 2, 3, 0, 0, 0)]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.opcode(), Opcode::TbufferStoreFormatXyzw);
    let m = inst.memory().unwrap();
    assert_eq!(
        (m.data_format(), m.number_format(), m.data_dwords()),
        (0xa, 5, 4)
    );
    assert!(m.typed() && m.formatted());
    assert_eq!(
        err(&[mtbuf(0x07, 0, 0), 1 << 21]),
        bad_opcode(Family::Mtbuf, 0x0f)
    );
}

#[test]
fn flat_segments_offsets_address_pairs_and_restrictions() {
    // SEG=0 masks the offset to 11 bits; VGPR pair continuation is addr + 1.
    let span = decoded(&[
        flat(0x0c, 0, 0, 0xfff) | (1 << 12) | (1 << 16) | (1 << 17),
        flat1(255, 0, 0x22, 9),
    ]);
    let inst = span.instruction(0).unwrap();
    let m = inst.memory().unwrap();
    assert_eq!((m.offset(), m.segment()), (0x7ff, 0));
    assert!(m.dlc() && m.glc() && m.slc());
    assert_eq!(inst.destination().register(), 9);
    assert_eq!(
        (inst.sources()[0].register(), inst.sources()[1].register()),
        (255, 256)
    );
    // Non-zero SEG sign-extends 12 bits and uses SADDR unless it is 0x7d/0x7f.
    let span = decoded(&[
        flat(0x1c, 2, 0, 0x800),
        flat1(4, 7, 0x22, 9),
        flat(0x24, 1, 0, 0x7ff),
        flat1(4, 7, 0x7f, 9),
    ]);
    let store = span.instruction(0).unwrap();
    assert_eq!(store.memory().unwrap().signed_offset(), -2048);
    assert_eq!(store.destination().register(), 7);
    assert_eq!(
        (store.sources()[1].kind(), store.sources()[1].register()),
        (OperandKind::Sgpr, 0x22)
    );
    let d16 = span.instruction(1).unwrap();
    assert_eq!(d16.destination().sdwa_select(), SdwaSelect::Word0);
    assert_eq!(
        (d16.sources()[1].kind(), d16.sources()[1].register()),
        (OperandKind::Vgpr, 5)
    );
    // LDS or SEG=3 are rejected before the opcode is checked.
    for header in [flat(0x00, 0, 1, 0), flat(0x00, 3, 0, 0)] {
        assert_eq!(
            err(&[header, 0]),
            reason(Family::Flat, 0, R::FlatModifiersOrSegment)
        );
    }
    assert_eq!(err(&[flat(0x00, 0, 0, 0), 0]), bad_opcode(Family::Flat, 0));
    assert_eq!(
        err(&[flat(0x0c, 1, 0, 0), flat1(0, 0, 110, 0)]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 110,
            destination: false
        }
    );
}

#[test]
fn ds_offsets_sources_partial_halves_and_field_restrictions() {
    let span = decoded(&[
        ds(0x37, 0, 3, 5),
        ds1(1, 2, 3, 4),
        ds(0x4f, 0, 3, 5),
        ds1(1, 2, 3, 4),
        ds(0x0d, 0, 0x34, 0x12),
        ds1(1, 2, 3, 4),
        ds(0xa0, 0, 0, 0),
        ds1(1, 2, 3, 4),
        ds(0xa1, 0, 0, 0),
        ds1(1, 2, 3, 4),
        ds(0xa7, 0, 0, 0),
        ds1(1, 2, 3, 4),
        ds(0x3e, 1, 0, 0),
        ds1(1, 2, 3, 4),
    ]);
    let offsets: Vec<_> = span
        .instructions()
        .map(|i| {
            (
                i.memory().unwrap().offset(),
                i.memory().unwrap().secondary_offset(),
            )
        })
        .collect();
    assert_eq!(&offsets[..3], [(12, 20), (1536, 2560), (0x1234, 0)]);
    let counts: Vec<_> = span.instructions().map(|i| i.sources().len()).collect();
    assert_eq!(counts, [1, 3, 2, 2, 2, 1, 0]);
    assert_eq!(
        span.instruction(1).unwrap().memory().unwrap().data_dwords(),
        4
    );
    assert_eq!(
        span.instruction(3).unwrap().sources()[1].sdwa_select(),
        SdwaSelect::Byte2
    );
    assert_eq!(
        span.instruction(4).unwrap().sources()[1].sdwa_select(),
        SdwaSelect::Word1
    );
    assert_eq!(
        span.instruction(5).unwrap().destination().sdwa_select(),
        SdwaSelect::Word1
    );
    assert!(span.instruction(6).unwrap().memory().unwrap().gds());
    // Ordered count: GDS required, wave-type field <= 1, bits 6..7 zero.
    let span = decoded(&[ds(0x3f, 1, 0xff, 0x13), ds1(1, 2, 3, 4)]);
    let m = span.instruction(0).unwrap().memory().unwrap();
    assert_eq!((m.offset(), m.secondary_offset()), (0xfc, 0x13));
    for (gds, offset1) in [(0, 0x10), (1, 0x20), (1, 0x40)] {
        assert_eq!(
            err(&[ds(0x3f, gds, 0, offset1), 0]),
            reason(Family::Ds, 0x3f, R::DsOrderedCountVariant)
        );
    }
    assert_eq!(
        err(&[ds(0x35, 0, 0, 0xe0), 0]),
        reason(Family::Ds, 0x35, R::DsSwizzleFft)
    );
    assert!(
        decode(
            &[ds(0x35, 0, 0xff, 0xdf), 0],
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    // FFT is checked first, then the GDS lane-operation rule.
    assert_eq!(
        err(&[ds(0x35, 1, 0, 0xe0), 0]),
        reason(Family::Ds, 0x35, R::DsSwizzleFft)
    );
    for op in [0x35, 0xb2, 0xb3, 0xb0, 0xb1] {
        assert_eq!(
            err(&[ds(op, 1, 0, 0), 0]),
            reason(Family::Ds, op as u16, R::DsLaneOperationGds)
        );
    }
    assert_eq!(
        err(&[ds(0xb0, 0, 0, 0), ds1(0, 5, 1, 0)]),
        reason(Family::Ds, 0xb0, R::DsWriteAddtidData1)
    );
    assert!(
        decode(
            &[ds(0xb0, 0, 0, 0), ds1(0, 5, 0, 0)],
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    assert_eq!(
        err(&[ds(0xb1, 0, 0, 0), ds1(0, 1, 0, 0)]),
        reason(Family::Ds, 0xb1, R::DsReadAddtidData)
    );
    assert_eq!(err(&[ds(0x02, 0, 0, 0), 0]), bad_opcode(Family::Ds, 0x02));
}

#[test]
fn mimg_nsa_lengths_addresses_and_truncation_need_the_full_body() {
    for nsa in 0..=3 {
        let words = Mimg {
            nsa,
            ..Mimg::new(0x00, 0xf)
        }
        .words();
        let span = decoded(&words);
        let inst = span.instruction(0).unwrap();
        assert_eq!(inst.raw_words(), &words[..]);
        let image = inst.image().unwrap();
        assert_eq!(usize::from(image.nsa_dwords()), nsa as usize);
        let expected: Vec<u8> = (0..nsa * 4).map(|i| 10 + i as u8).collect();
        assert_eq!(image.nsa_addresses(), &expected[..]);
        for available in 1..words.len() {
            assert_eq!(
                err(&words[..available]),
                truncated(Family::Mimg, words.len(), available)
            );
        }
    }
}

#[test]
fn mimg_sample_gather_load_fields_and_address_components() {
    let words = Mimg {
        glc: 1,
        slc: 1,
        tfe: 1,
        lwe: 1,
        r128: 1,
        a16: 1,
        vaddr: 5,
        vdata: 6,
        srsrc: 2,
        ssamp: 3,
        ..Mimg::new(0x3b, 0b1011)
    }
    .words();
    let span = decoded(&words);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.opcode(), Opcode::ImageSample);
    assert_eq!(inst.mnemonic(), "image_sample_c_d_cl_o");
    let image = inst.image().unwrap();
    let flags = image.sample_flags();
    for flag in [
        ImageSampleFlags::COMPARE,
        ImageSampleFlags::DERIVATIVE,
        ImageSampleFlags::LOD_CLAMP,
        ImageSampleFlags::OFFSET,
        ImageSampleFlags::A16,
    ] {
        assert!(flags.contains(flag));
    }
    assert!(!flags.contains(ImageSampleFlags::BIAS));
    // 2D: two coordinates + offset + compare + clamp + 2x2 derivatives.
    assert_eq!(image.address_components(), 9);
    assert_eq!(
        (image.dmask(), image.data_components(), image.dimension()),
        (0b1011, 3, ImageDimension::Dim2D)
    );
    assert!(image.r128());
    let m = inst.memory().unwrap();
    assert!(m.glc() && m.slc() && m.tfe() && m.lwe());
    assert_eq!((m.data_dwords(), m.data_bits()), (3, 32));
    let regs: Vec<_> = inst
        .sources()
        .iter()
        .map(|s| (s.kind(), s.register()))
        .collect();
    assert_eq!(
        regs,
        [
            (OperandKind::Vgpr, 5),
            (OperandKind::Sgpr, 8),
            (OperandKind::Sgpr, 12)
        ]
    );
    assert_eq!(inst.destination().register(), 6);
    // Cube (3) and array (5) both map to Dim2DArray; raw bits remain.
    for (dim, expected, components) in [
        (3, ImageDimension::Dim2DArray, 7),
        (5, ImageDimension::Dim2DArray, 7),
        (2, ImageDimension::Dim3D, 9),
        (7, ImageDimension::Dim2DMsaaArray, 4),
    ] {
        let op = if dim == 7 { 0x00 } else { 0x68 };
        let span = decoded(
            &Mimg {
                dim,
                ..Mimg::new(op, 1)
            }
            .words(),
        );
        let image = *span.instruction(0).unwrap().image().unwrap();
        assert_eq!(
            (
                image.dimension(),
                image.dimension_bits(),
                image.address_components()
            ),
            (expected, dim as u8, components)
        );
    }
    // Misc opcodes: resinfo has one address, mip forms add one.
    for (op, components) in [(0x0e, 1), (0x01, 3), (0x09, 3), (0x08, 2), (0x60, 2)] {
        let span = decoded(&Mimg::new(op, 1).words());
        assert_eq!(
            span.instruction(0)
                .unwrap()
                .image()
                .unwrap()
                .address_components(),
            components
        );
    }
    // Gathers return four components and need exactly one dmask bit.
    let span = decoded(
        &Mimg {
            d16: 1,
            ..Mimg::new(0x61, 0b0100)
        }
        .words(),
    );
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.opcode(), Opcode::ImageGather4h);
    assert_eq!(inst.image().unwrap().data_components(), 4);
    assert_eq!(
        (
            inst.memory().unwrap().data_dwords(),
            inst.memory().unwrap().data_bits()
        ),
        (2, 16)
    );
    assert_eq!(
        err(&Mimg::new(0x44, 0b0011).words()),
        reason(Family::Mimg, 0x44, R::MimgGatherDmask)
    );
    assert_eq!(
        err(&Mimg::new(0x44, 0).words()),
        reason(Family::Mimg, 0x44, R::MimgGatherDmask)
    );
    // Empty dmask still counts one component for non-gathers.
    assert_eq!(
        decoded(&Mimg::new(0x00, 0).words())
            .instruction(0)
            .unwrap()
            .image()
            .unwrap()
            .data_components(),
        1
    );
}

#[test]
fn mimg_atomic_d16_and_bvh_restrictions_keep_the_last_source_reason() {
    let bits = |m: Mimg| {
        decoded(&m.words())
            .instruction(0)
            .unwrap()
            .memory()
            .unwrap()
            .data_bits()
    };
    assert_eq!(bits(Mimg::new(0x11, 1)), 32);
    assert_eq!(bits(Mimg::new(0x11, 3)), 64);
    assert_eq!(bits(Mimg::new(0x10, 3)), 32);
    assert_eq!(
        err(&Mimg::new(0x12, 3).words()),
        reason(Family::Mimg, 0x12, R::MimgAtomic64)
    );
    assert_eq!(
        err(&Mimg::new(0x10, 0xf).words()),
        reason(Family::Mimg, 0x10, R::MimgAtomic64)
    );
    assert_eq!(
        err(&Mimg::new(0x10, 1).words()),
        reason(Family::Mimg, 0x10, R::MimgAtomicDmask)
    );
    assert_eq!(
        err(&Mimg {
            d16: 1,
            ..Mimg::new(0x0e, 1)
        }
        .words()),
        reason(Family::Mimg, 0x0e, R::MimgD16)
    );
    // D16 is checked last and replaces an earlier atomic reason.
    assert_eq!(
        err(&Mimg {
            d16: 1,
            ..Mimg::new(0x10, 1)
        }
        .words()),
        reason(Family::Mimg, 0x10, R::MimgD16)
    );
    // BVH: address dwords and restriction chain.
    let bvh = |op, a16| Mimg {
        r128: 1,
        a16,
        ..Mimg::new(op, 0xf)
    };
    for (op, a16, dwords) in [(0xe6, 0, 11), (0xe6, 1, 8), (0xe7, 0, 12), (0xe7, 1, 9)] {
        let span = decoded(&bvh(op, a16).words());
        let inst = span.instruction(0).unwrap();
        assert!(inst.is_bvh_intersect());
        assert_eq!(inst.image().unwrap().address_components(), dwords);
        assert_eq!(inst.sources().len(), 3);
    }
    assert_eq!(
        err(&Mimg {
            dmask: 7,
            r128: 0,
            ..bvh(0xe6, 0)
        }
        .words()),
        reason(Family::Mimg, 0xe6, R::BvhDmask)
    );
    assert_eq!(
        err(&Mimg {
            r128: 0,
            ..bvh(0xe6, 0)
        }
        .words()),
        reason(Family::Mimg, 0xe6, R::BvhR128)
    );
    assert_eq!(
        err(&Mimg {
            lwe: 1,
            ..bvh(0xe6, 0)
        }
        .words()),
        reason(Family::Mimg, 0xe6, R::BvhTfeLwe)
    );
    assert_eq!(
        err(&Mimg {
            nsa: 2,
            ..bvh(0xe6, 0)
        }
        .words()),
        reason(Family::Mimg, 0xe6, R::BvhNsaAddresses)
    );
    assert!(
        decode(
            &Mimg {
                nsa: 3,
                ..bvh(0xe6, 0)
            }
            .words(),
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    assert!(
        decode(
            &Mimg {
                nsa: 2,
                ..bvh(0xe7, 1)
            }
            .words(),
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    assert_eq!(
        err(&Mimg {
            d16: 1,
            dmask: 7,
            ..bvh(0xe6, 0)
        }
        .words()),
        reason(Family::Mimg, 0xe6, R::MimgD16)
    );
    // An unmapped opcode still decodes resources; an operand abort wins.
    assert_eq!(
        err(&Mimg::new(0x02, 1).words()),
        bad_opcode(Family::Mimg, 0x02)
    );
    assert_eq!(
        err(&Mimg {
            ssamp: 28,
            ..Mimg::new(0x02, 1)
        }
        .words()),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 112,
            destination: false
        }
    );
    assert!(
        !decoded(&Mimg::new(0x00, 1).words())
            .instruction(0)
            .unwrap()
            .is_bvh_intersect()
    );
}

#[test]
fn image_address_layout_helpers_match_the_source_packing() {
    let all32 = ImageSampleFlags::OFFSET.union(ImageSampleFlags::COMPARE);
    for component in 0..4u8 {
        let layout = image_address_component_layout(all32, component);
        assert_eq!(
            (layout.bit_offset, layout.bit_width),
            (u32::from(component) * 32, 32)
        );
    }
    assert_eq!(image_address_dword_count(all32, 4), 4);
    assert_eq!(image_address_dword_count(all32, 0), 0);
    // A16 with offset (32), bias (16), compare (32, aligned), then 16-bit coordinates.
    let a16 = ImageSampleFlags::A16
        .union(ImageSampleFlags::OFFSET)
        .union(ImageSampleFlags::BIAS)
        .union(ImageSampleFlags::COMPARE);
    let layout: Vec<_> = (0..5)
        .map(|c| image_address_component_layout(a16, c))
        .map(|l| (l.bit_offset, l.bit_width))
        .collect();
    assert_eq!(layout, [(0, 32), (32, 16), (64, 32), (96, 16), (112, 16)]);
    assert_eq!(image_address_dword_count(a16, 5), 4);
    assert_eq!(image_address_dword_count(ImageSampleFlags::A16, 3), 2);
    // The largest representable index stays bounded and does not overflow.
    let last = image_address_component_layout(ImageSampleFlags::NONE, u8::MAX);
    assert_eq!((last.bit_offset, last.bit_width), (255 * 32, 32));
}
