// SPDX-License-Identifier: GPL-2.0-only
// Modified 2026 for PentaX: checked Rust outlines, bounded grayscale coverage,
// and a licensed bundled Hack fallback in place of host font searches/VGA.
// Adapted Font metrics/scaling policy from KytyPS5-Performance
// 209dffe9032d72749257ce18680d506b891284de, src/libs/libFont.cpp
// Git blob c3b58f805f0781105a9a7cf216bf4750b3e57d5e. See NOTICE.

use crate::{Failure, budget::Budget};
use skrifa::raw::ps::{
    cff::{CffFontRef, blend::BlendState, index::Index},
    cs::{self, CharstringContext, CharstringKind, NopFilterSink, TransformSink},
    error::Error,
};
use skrifa::raw::{
    FontRead,
    ps::cff::dict,
    tables::{cff::Cff, cff2::Cff2, variations::ItemVariationStore},
};
use skrifa::{
    FontRef, GlyphId, MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, Hinting, OutlineGlyphCollection, OutlineGlyphFormat, OutlinePen},
};
use std::cell::Cell;

pub(super) const STRIDE: usize = 128;
pub(super) const IMAGE_BYTES: usize = STRIDE * STRIDE;
const MAX_FONT: usize = 8 * 1024 * 1024;
const MAX_EDGES: usize = 16_384;
const MAX_NODES: usize = 1024;
const MAX_POINTS: usize = 16_384;
const CFF_BYTES: usize = 262_144;
const MAX_DRAW_MEMORY: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    pub(super) values: [f32; 8],
}
impl Metrics {
    pub(super) fn bytes(&self) -> [u8; 32] {
        let mut bytes = [0; 32];
        for (index, value) in self.values.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
pub(super) struct GlyphImage {
    pub(super) metrics: Metrics,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: Vec<u8>,
}
impl GlyphImage {
    pub(super) fn metadata_bytes(&self) -> usize {
        self.pixels.capacity()
    }
}

fn bad() -> Failure {
    Failure::Unsupported("invalid font outline")
}
fn add(a: usize, b: usize) -> Result<usize, Failure> {
    super::add(a, b)
}
fn buffer(length: usize, held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    super::admit(add(held, length)?, budget)?;
    let bytes = budget.buffer_held(length, held)?;
    super::admit(add(held, bytes.capacity())?, budget)?;
    Ok(bytes)
}
fn be16(bytes: &[u8], at: usize) -> Result<u16, Failure> {
    Ok(u16::from_be_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ))
}
fn be32(bytes: &[u8], at: usize) -> Result<u32, Failure> {
    Ok(u32::from_be_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ))
}
fn signed(bytes: &[u8], at: usize) -> Result<i16, Failure> {
    Ok(be16(bytes, at)? as i16)
}

struct Face<'a> {
    data: &'a [u8],
    tables: usize,
}
impl<'a> Face<'a> {
    fn new(input: Option<&'a [u8]>, budget: &mut Budget) -> Result<Self, Failure> {
        let data = input
            .filter(|bytes| {
                bytes.len() >= 12 && matches!(&bytes[..4], b"\0\x01\0\0" | b"true" | b"OTTO")
            })
            .unwrap_or(epaint_default_fonts::HACK_REGULAR);
        if data.len() > MAX_FONT {
            return Err(Failure::Limit("font data bytes"));
        }
        let tables = usize::from(be16(data, 4)?);
        if tables == 0 || tables > 128 || 12 + tables * 16 > data.len() {
            return Err(bad());
        }
        budget.work(tables as u64 * tables as u64 + 4096)?;
        let face = Self { data, tables };
        // Reject duplicate and out-of-file records before a library lookup.
        for i in 0..tables {
            let at = 12 + i * 16;
            let start = be32(data, at + 8)? as usize;
            let length = be32(data, at + 12)? as usize;
            if start.checked_add(length).is_none_or(|end| end > data.len()) {
                return Err(bad());
            }
            for j in 0..i {
                if data[at..at + 4] == data[12 + j * 16..16 + j * 16] {
                    return Err(bad());
                }
            }
        }
        let cmap = face.table(b"cmap")?.ok_or_else(bad)?;
        if be16(cmap, 2)? > 64 {
            return Err(Failure::Limit("font cmap records"));
        }
        Ok(face)
    }
    fn table(&self, tag: &[u8; 4]) -> Result<Option<&'a [u8]>, Failure> {
        for i in 0..self.tables {
            let at = 12 + i * 16;
            if &self.data[at..at + 4] == tag {
                let start = be32(self.data, at + 8)? as usize;
                let length = be32(self.data, at + 12)? as usize;
                return Ok(Some(&self.data[start..start + length]));
            }
        }
        Ok(None)
    }
    fn required(&self, tag: &[u8; 4]) -> Result<&'a [u8], Failure> {
        self.table(tag)?.ok_or_else(bad)
    }
    fn scale(&self, scale_h: f32) -> Result<(f32, f32, u16), Failure> {
        let height = pixel_height(scale_h)?;
        let hhea = self.required(b"hhea")?;
        let ascent = f32::from(signed(hhea, 4)?);
        let descent = f32::from(signed(hhea, 6)?);
        let upem = be16(self.required(b"head")?, 18)?;
        if ascent <= descent || upem == 0 {
            return Err(bad());
        }
        Ok((height / (ascent - descent), height, upem))
    }
    fn glyph(&self, glyph: u32) -> Result<&'a [u8], Failure> {
        let count = u32::from(be16(self.required(b"maxp")?, 4)?);
        if glyph >= count {
            return Err(bad());
        }
        let loca = self.required(b"loca")?;
        let long = match signed(self.required(b"head")?, 50)? {
            0 => false,
            1 => true,
            _ => return Err(bad()),
        };
        let offset = |index: u32| -> Result<usize, Failure> {
            if long {
                Ok(be32(loca, index as usize * 4)? as usize)
            } else {
                Ok(usize::from(be16(loca, index as usize * 2)?) * 2)
            }
        };
        let begin = offset(glyph)?;
        let end = offset(glyph + 1)?;
        self.required(b"glyf")?.get(begin..end).ok_or_else(bad)
    }
    fn preflight_glyf(
        &self,
        glyph: u32,
        depth: usize,
        nodes: &mut usize,
        points: &mut usize,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        *nodes += 1;
        if *nodes > MAX_NODES || depth > 16 {
            return Err(Failure::Limit("font composite expansion"));
        }
        budget.work(32)?;
        let bytes = self.glyph(glyph)?;
        if bytes.is_empty() {
            return Ok(());
        }
        let contours = signed(bytes, 0)?;
        if contours >= 0 {
            let count = contours as usize;
            let n = if count == 0 {
                0
            } else {
                usize::from(be16(bytes, 10 + (count - 1) * 2)?) + 1
            };
            *points = points
                .checked_add(n)
                .ok_or(Failure::Limit("font point count"))?;
            if *points > MAX_POINTS {
                return Err(Failure::Limit("font point count"));
            }
            budget.work(bytes.len() as u64 + n as u64 * 32)?;
            return Ok(());
        }
        let mut cursor = 10;
        loop {
            let flags = be16(bytes, cursor)?;
            let component = be16(bytes, cursor + 2)?;
            cursor += 4;
            cursor += if flags & 1 != 0 { 4 } else { 2 };
            cursor += if flags & 8 != 0 {
                2
            } else if flags & 64 != 0 {
                4
            } else if flags & 128 != 0 {
                8
            } else {
                0
            };
            if cursor > bytes.len() {
                return Err(bad());
            }
            self.preflight_glyf(u32::from(component), depth + 1, nodes, points, budget)?;
            if flags & 32 == 0 {
                break;
            }
        }
        Ok(())
    }
    fn advance(&self, glyph: u32, scale: f32) -> Result<f32, Failure> {
        let count = usize::from(be16(self.required(b"hhea")?, 34)?);
        if count == 0 {
            return Err(bad());
        }
        Ok(f32::from(be16(
            self.required(b"hmtx")?,
            (glyph as usize).min(count - 1) * 4,
        )?) * scale)
    }
}

fn pixel_height(scale_h: f32) -> Result<f32, Failure> {
    if !scale_h.is_finite() {
        return Err(Failure::Unsupported("nonfinite font scale"));
    }
    let value = if scale_h > 1.0 { scale_h } else { 16.0 };
    // C++ converts before clamp; avoid undefined out-of-range conversion.
    if value + 0.5 >= i32::MAX as f32 {
        return Err(Failure::Unsupported("font scale conversion"));
    }
    Ok(((value + 0.5) as i32).clamp(8, 128) as f32)
}

#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}
impl Point {
    fn mid(self, other: Self) -> Self {
        Self {
            x: (self.x + other.x) * 0.5,
            y: (self.y + other.y) * 0.5,
        }
    }
}
#[derive(Clone, Copy)]
struct Edge {
    a: Point,
    b: Point,
}
struct Pen {
    edges: Vec<Edge>,
    current: Point,
    first: Point,
    bounds: Option<[f32; 4]>,
    failed: bool,
    operations: usize,
}
impl Pen {
    fn new(held: usize, budget: &mut Budget) -> Result<Self, Failure> {
        // The sink cannot stop the external outline walk through a Result.
        // Admit its complete bounded subdivision envelope before callbacks.
        budget.work(MAX_EDGES as u64 * 4 * 32)?;
        let expected = MAX_EDGES * std::mem::size_of::<Edge>();
        budget.scratch(add(held, expected)?)?;
        let mut edges = Vec::new();
        edges
            .try_reserve_exact(MAX_EDGES)
            .map_err(|_| Failure::Allocation)?;
        budget.scratch(add(held, edges.capacity() * std::mem::size_of::<Edge>())?)?;
        budget.record_scratch(add(held, edges.capacity() * std::mem::size_of::<Edge>())?);
        Ok(Self {
            edges,
            current: Point { x: 0.0, y: 0.0 },
            first: Point { x: 0.0, y: 0.0 },
            bounds: None,
            failed: false,
            operations: 0,
        })
    }
    fn capacity_bytes(&self) -> usize {
        self.edges.capacity() * std::mem::size_of::<Edge>()
    }
    fn point(&mut self, point: Point) {
        if !point.x.is_finite()
            || !point.y.is_finite()
            || point.x.abs() > 1_000_000.0
            || point.y.abs() > 1_000_000.0
        {
            self.failed = true;
            return;
        }
        if let Some(bounds) = &mut self.bounds {
            bounds[0] = bounds[0].min(point.x);
            bounds[1] = bounds[1].min(point.y);
            bounds[2] = bounds[2].max(point.x);
            bounds[3] = bounds[3].max(point.y);
        } else {
            self.bounds = Some([point.x, point.y, point.x, point.y]);
        }
    }
    fn edge(&mut self, a: Point, b: Point) {
        self.operations += 1;
        if self.failed || self.edges.len() == MAX_EDGES || self.operations > MAX_EDGES * 4 {
            self.failed = true;
            return;
        }
        self.point(a);
        self.point(b);
        self.edges.push(Edge { a, b });
    }
    fn quad(&mut self, a: Point, b: Point, c: Point, depth: u8) {
        if self.failed {
            return;
        }
        let error = (a.x - 2.0 * b.x + c.x)
            .abs()
            .max((a.y - 2.0 * b.y + c.y).abs());
        if error <= 0.125 {
            self.edge(a, c);
            return;
        }
        if depth == 12 {
            self.failed = true;
            return;
        }
        let ab = a.mid(b);
        let bc = b.mid(c);
        let middle = ab.mid(bc);
        self.quad(a, ab, middle, depth + 1);
        self.quad(middle, bc, c, depth + 1);
    }
    fn cubic(&mut self, a: Point, b: Point, c: Point, d: Point, depth: u8) {
        if self.failed {
            return;
        }
        let error = (a.x - 2.0 * b.x + c.x)
            .abs()
            .max((a.y - 2.0 * b.y + c.y).abs())
            .max((b.x - 2.0 * c.x + d.x).abs())
            .max((b.y - 2.0 * c.y + d.y).abs());
        if error <= 0.125 {
            self.edge(a, d);
            return;
        }
        if depth == 12 {
            self.failed = true;
            return;
        }
        let ab = a.mid(b);
        let bc = b.mid(c);
        let cd = c.mid(d);
        let abc = ab.mid(bc);
        let bcd = bc.mid(cd);
        let middle = abc.mid(bcd);
        self.cubic(a, ab, abc, middle, depth + 1);
        self.cubic(middle, bcd, cd, d, depth + 1);
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.current = Point { x, y: -y };
        self.first = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let next = Point { x, y: -y };
        self.edge(self.current, next);
        self.current = next;
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let next = Point { x, y: -y };
        self.quad(self.current, Point { x: cx, y: -cy }, next, 0);
        self.current = next;
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let next = Point { x, y: -y };
        self.cubic(
            self.current,
            Point { x: ax, y: -ay },
            Point { x: bx, y: -by },
            next,
            0,
        );
        self.current = next;
    }
    fn close(&mut self) {
        self.edge(self.current, self.first);
        self.current = self.first;
    }
}

// The library's recursion limit alone does not bound repeated subroutine
// expansion. Charge every returned charstring span before its evaluation.
struct CffBudget<'a> {
    inner: (&'a [u8], &'a Index<'a>, &'a Index<'a>, &'a Index<'a>),
    remaining: Cell<usize>,
}
impl CffBudget<'_> {
    fn span<'a>(&self, value: &'a [u8]) -> Result<&'a [u8], Error> {
        let left = self
            .remaining
            .get()
            .checked_sub(value.len())
            .ok_or(Error::CharstringNestingDepthLimitExceeded)?;
        self.remaining.set(left);
        Ok(value)
    }
}
impl CharstringContext for CffBudget<'_> {
    fn kind(&self) -> CharstringKind {
        CharstringKind::Type2
    }
    fn global_subr(&self, index: i32) -> Result<&[u8], Error> {
        self.span(self.inner.global_subr(index)?)
    }
    fn subr(&self, index: i32) -> Result<&[u8], Error> {
        self.span(self.inner.subr(index)?)
    }
    fn seac_components(&self, base: i32, accent: i32) -> Result<[&[u8]; 2], Error> {
        let [a, b] = self.inner.seac_components(base, accent)?;
        Ok([self.span(a)?, self.span(b)?])
    }
}

// Inspect DICT declarations without executing Private DICT blends. The selected
// exact private range must be admitted before CffFontRef::subfont constructs its
// BlendState and parses that range. Top/Font DICTs have no blend state.
fn cff_private_len(bytes: &[u8], cff2: bool, selected: u16) -> Result<usize, Failure> {
    let top = if cff2 {
        Cff2::read(bytes.into()).map_err(|_| bad())?.top_dict_data()
    } else {
        Cff::read(bytes.into())
            .map_err(|_| bad())?
            .top_dicts()
            .get(0)
            .map_err(|_| bad())?
    };
    let mut private = 0..0;
    let mut array = None;
    for entry in dict::entries(top, None) {
        match entry.map_err(|_| bad())? {
            dict::Entry::PrivateDictRange(range) => private = range,
            dict::Entry::FdArrayOffset(offset) => array = Some(offset),
            _ => {}
        }
    }
    if let Some(offset) = array {
        let array = Index::new(bytes.get(offset..).ok_or_else(bad)?, cff2).map_err(|_| bad())?;
        let font_dict = array.get(usize::from(selected)).map_err(|_| bad())?;
        private = 0..0;
        for entry in dict::entries(font_dict, None) {
            if let dict::Entry::PrivateDictRange(range) = entry.map_err(|_| bad())? {
                private = range;
            }
        }
    }
    Ok(bytes.get(private).ok_or_else(bad)?.len())
}

fn cff_blend_work(
    store: Option<&ItemVariationStore<'_>>,
    private_bytes: usize,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let Some(store) = store else {
        return budget.work(CFF_BYTES as u64 * 16);
    };
    // vsindex can select any declared variation data, and zero-target blend
    // still walks every region scalar. Default coordinates do not avoid that
    // axis walk. Count all declarations before any BlendState construction.
    budget.work(u64::from(store.item_variation_data_count()) * 8 + 1)?;
    let axes = u64::from(
        store
            .variation_region_list()
            .map_err(|_| bad())?
            .axis_count(),
    );
    let mut regions = 0_u64;
    for entry in store.item_variation_data().iter().flatten() {
        regions = regions.max(entry.map_err(|_| bad())?.region_indexes().len() as u64);
    }
    let per_byte = axes
        .checked_add(8)
        .and_then(|value| value.checked_mul(regions))
        .and_then(|value| value.checked_add(513 * 8 + 64))
        .ok_or(Failure::Limit("font CFF variation work"))?;
    let expanded = (CFF_BYTES as u64)
        .checked_add(private_bytes as u64)
        .and_then(|value| value.checked_mul(per_byte))
        .ok_or(Failure::Limit("font CFF variation work"))?;
    // Subfont Private DICT and final charstring each build initial cached
    // scalars. Every later vsindex is already included in the byte envelope.
    let initial = (axes + 8)
        .checked_mul(regions.min(16))
        .and_then(|value| value.checked_mul(2))
        .ok_or(Failure::Limit("font CFF variation work"))?;
    budget.work(
        expanded
            .checked_add(initial)
            .ok_or(Failure::Limit("font CFF variation work"))?,
    )
}

fn cff_draw(
    face: &Face<'_>,
    glyph: GlyphId,
    ppem: f32,
    pen: &mut Pen,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let (bytes, cff2) = if let Some(bytes) = face.table(b"CFF2")? {
        (bytes, true)
    } else {
        (face.required(b"CFF ")?, false)
    };
    // Dictionary, FDSelect and INDEX traversal is bounded by retained bytes;
    // expansion has an independent cumulative limit, including repeated calls.
    budget.work(bytes.len() as u64 * 8)?;
    let upem = i32::from(be16(face.required(b"head")?, 18)?);
    let font = if cff2 {
        CffFontRef::new_cff2(bytes, Some(upem))
    } else {
        CffFontRef::new_cff(bytes, 0, Some(upem))
    }
    .map_err(|_| bad())?;
    let selected = font.subfont_index(glyph).ok_or_else(bad)?;
    let private_bytes = cff_private_len(bytes, cff2, selected)?;
    cff_blend_work(font.var_store(), private_bytes, budget)?;
    let subfont = font.subfont(selected, &[]).map_err(|_| bad())?;
    let local = if subfont.subrs_offset() != 0 {
        Index::new(
            bytes
                .get(subfont.subrs_offset() as usize..)
                .ok_or_else(bad)?,
            cff2,
        )
        .map_err(|_| bad())?
    } else {
        Index::Empty
    };
    let context = CffBudget {
        inner: (bytes, font.charstrings(), font.global_subrs(), &local),
        remaining: Cell::new(CFF_BYTES),
    };
    let program = context
        .span(
            font.charstrings()
                .get(glyph.to_u32() as usize)
                .map_err(|_| bad())?,
        )
        .map_err(|_| bad())?;
    let blend = font
        .var_store()
        .map(|store| BlendState::new(store.clone(), &[], subfont.vs_index()))
        .transpose()
        .map_err(|_| bad())?;
    let mut sink = NopFilterSink::new(pen);
    let mut transform = TransformSink::new(&mut sink, font.transform(&subfont, Some(ppem)));
    cs::evaluate(&context, blend, program, &mut transform).map_err(|_| bad())?;
    Ok(())
}

fn outline(
    data: Option<&[u8]>,
    scale_h: f32,
    code: u32,
    held: usize,
    budget: &mut Budget,
) -> Result<(Pen, Metrics), Failure> {
    let face = Face::new(data, budget)?;
    let (scale, height, upem) = face.scale(scale_h)?;
    let font = FontRef::new(face.data).map_err(|_| bad())?;
    let cmap = font.charmap();
    let glyph = cmap
        .map(code)
        .filter(|id| id.to_u32() != 0)
        .or_else(|| cmap.map(u32::from(b'?')))
        .ok_or_else(bad)?;
    let mut pen = Pen::new(held, budget)?;
    let mut bounds = None;
    if face.table(b"glyf")?.is_some() {
        let mut nodes = 0;
        let mut points = 0;
        face.preflight_glyf(glyph.to_u32(), 0, &mut nodes, &mut points, budget)?;
        let bytes = face.glyph(glyph.to_u32())?;
        if !bytes.is_empty() {
            bounds = Some([
                f32::from(signed(bytes, 2)?) * scale,
                -f32::from(signed(bytes, 8)?) * scale,
                f32::from(signed(bytes, 6)?) * scale,
                -f32::from(signed(bytes, 4)?) * scale,
            ]);
        }
        let outlines =
            OutlineGlyphCollection::with_format(&font, OutlineGlyphFormat::Glyf).ok_or_else(bad)?;
        let glyph = outlines.get(glyph).ok_or_else(bad)?;
        let required = glyph.draw_memory_size(Hinting::None);
        if required > MAX_DRAW_MEMORY {
            return Err(Failure::Limit("font draw memory"));
        }
        let mut scratch = buffer(required, add(held, pen.capacity_bytes())?, budget)?;
        glyph
            .draw(
                DrawSettings::unhinted(Size::new(f32::from(upem) * scale), LocationRef::default())
                    .with_memory(Some(scratch.as_mut_slice())),
                &mut pen,
            )
            .map_err(|_| bad())?;
    } else {
        cff_draw(&face, glyph, f32::from(upem) * scale, &mut pen, budget)?;
    }
    if pen.failed {
        return Err(Failure::Limit("font outline edges"));
    }
    let bounds = bounds.or(pen.bounds).unwrap_or([0.0; 4]);
    let x0 = bounds[0].floor();
    let y0 = bounds[1].floor();
    let width = (bounds[2].ceil() - x0).max(0.0);
    let tall = (bounds[3].ceil() - y0).max(0.0);
    let metrics = Metrics {
        values: [
            width,
            tall,
            x0,
            -y0,
            face.advance(glyph.to_u32(), scale)?.max(width),
            0.0,
            0.0,
            height,
        ],
    };
    Ok((pen, metrics))
}

pub(super) fn metrics(
    data: Option<&[u8]>,
    scale_h: f32,
    code: u32,
    held: usize,
    budget: &mut Budget,
) -> Result<Metrics, Failure> {
    outline(data, scale_h, code, held, budget).map(|(_, metrics)| metrics)
}

pub(super) fn horizontal_layout(
    data: Option<&[u8]>,
    scale_h: f32,
    _held: usize,
    budget: &mut Budget,
) -> Result<[f32; 3], Failure> {
    let face = Face::new(data, budget)?;
    let (scale, _, _) = face.scale(scale_h)?;
    let hhea = face.required(b"hhea")?;
    let ascent = f32::from(signed(hhea, 4)?);
    let descent = f32::from(signed(hhea, 6)?);
    let gap = f32::from(signed(hhea, 8)?);
    let line = (ascent - descent + gap) * scale;
    Ok([ascent * scale, line, line])
}

#[derive(Clone, Copy)]
struct Crossing {
    x: f32,
    delta: i32,
}

pub(super) fn rasterize(
    data: Option<&[u8]>,
    scale_h: f32,
    code: u32,
    held: usize,
    budget: &mut Budget,
) -> Result<GlyphImage, Failure> {
    let (pen, metrics) = outline(data, scale_h, code, held, budget)?;
    let width = metrics.values[0].clamp(0.0, 128.0) as u32;
    let height = metrics.values[1].clamp(0.0, 128.0) as u32;
    let held = add(held, pen.capacity_bytes())?;
    let mut pixels = buffer(IMAGE_BYTES, held, budget)?;
    let held = add(held, pixels.capacity())?;
    let mut crossings = Vec::<Crossing>::new();
    let expected = pen.edges.len() * std::mem::size_of::<Crossing>();
    budget.scratch(add(held, expected)?)?;
    crossings
        .try_reserve_exact(pen.edges.len())
        .map_err(|_| Failure::Allocation)?;
    budget.scratch(add(
        held,
        crossings.capacity() * std::mem::size_of::<Crossing>(),
    )?)?;
    budget.record_scratch(add(
        held,
        crossings.capacity() * std::mem::size_of::<Crossing>(),
    )?);
    let steps = (pen.edges.len() as u64 + 1).ilog2() as u64 + 1;
    budget.work(
        u64::from(height) * 4 * (pen.edges.len() as u64 * (steps + 8) + u64::from(width) * 4 + 1),
    )?;
    for y in 0..height as usize {
        for sub_y in 0..4 {
            crossings.clear();
            let sample_y = y as f32 - metrics.values[3] + (sub_y as f32 + 0.5) * 0.25;
            for edge in &pen.edges {
                let (a, b) = (edge.a, edge.b);
                if (a.y <= sample_y && sample_y < b.y) || (b.y <= sample_y && sample_y < a.y) {
                    let x = a.x + (sample_y - a.y) * (b.x - a.x) / (b.y - a.y);
                    crossings.push(Crossing {
                        x,
                        delta: if b.y > a.y { 1 } else { -1 },
                    });
                }
            }
            crossings.sort_unstable_by(|a, b| a.x.total_cmp(&b.x));
            let mut cursor = 0;
            let mut winding = 0_i32;
            for x in 0..width as usize {
                for sub_x in 0..4 {
                    let sample_x = x as f32 + metrics.values[2] + (sub_x as f32 + 0.5) * 0.25;
                    while cursor < crossings.len() && crossings[cursor].x <= sample_x {
                        winding += crossings[cursor].delta;
                        cursor += 1;
                    }
                    if winding != 0 {
                        pixels[y * STRIDE + x] += 1;
                    }
                }
            }
        }
    }
    for value in &mut pixels {
        *value = ((u16::from(*value) * 255 + 8) / 16) as u8;
    }
    Ok(GlyphImage {
        metrics,
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
