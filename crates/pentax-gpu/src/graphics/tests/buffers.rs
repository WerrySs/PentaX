// SPDX-License-Identifier: GPL-2.0-only
//! Original CPU preparation consumers for actual direct graphics resources.
use super::*;

const VERTEX_BUFFER: u64 = 0x30000;
const PIXEL_BUFFER: u64 = 0x40000;

fn vertex_base(base: u32) -> u32 {
    if base == 0 { 8 } else { base }
}

fn program(vertex: bool, base: u32, scalar: bool) -> Vec<u32> {
    let mut words = if scalar {
        let mut words = vec![
            0xbe80_0000 | (40 << 16) | (3 << 8) | 128,
            0xf400_0000 | (0x0a << 18) | (32 << 6) | (base / 2),
            40 << 25,
        ];
        words.extend((0..4).map(|lane| unary(1, lane, 32 + lane)));
        words
    } else {
        vec![0xe000_0000 | (0x0e << 18), (128 << 24) | ((base / 4) << 16)]
    };
    words.extend(export(if vertex { 12 } else { 0 }, [0, 1, 2, 3]));
    words.push(END);
    words
}

struct Buffers {
    source: Source,
    ranges: [(u64, RefCell<Vec<u8>>); 2],
    reads: RefCell<Vec<(u64, usize)>>,
    refused: Cell<bool>,
}
impl Buffers {
    fn new(base: u32, scalar: bool) -> Self {
        let mut source = Source::new();
        source.vertex = program(true, vertex_base(base), scalar);
        source.pixel = program(false, base, scalar);
        Self {
            source,
            ranges: [
                (
                    VERTEX_BUFFER,
                    RefCell::new(bytes(&[0.0_f32.to_bits(), 0, 0, 1.0_f32.to_bits()])),
                ),
                (
                    PIXEL_BUFFER,
                    RefCell::new(bytes(&[0.25_f32.to_bits(), 0, 0, 1.0_f32.to_bits()])),
                ),
            ],
            reads: RefCell::new(Vec::new()),
            refused: Cell::new(false),
        }
    }
}
impl Memory for Buffers {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.source.read(address, output)
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), MemoryError> {
        if !written {
            assert!(read);
            self.reads.borrow_mut().push((address, output.len()));
            if self.refused.get() {
                return Err(MemoryError::ReadDenied);
            }
            if let Some((_, bytes)) = self.ranges.iter().find(|(base, _)| *base == address) {
                let bytes = bytes.borrow();
                if output.len() <= bytes.len() {
                    output.copy_from_slice(&bytes[..output.len()]);
                    return Ok(());
                }
            }
            return Err(MemoryError::Unavailable);
        }
        self.source.snapshot_buffer(address, output, read, written)
    }
}

fn descriptor(address: u64, size: u32) -> [u32; 4] {
    [address as u32, (address >> 32) as u32, size, 0]
}

fn draw(base: u32, vertex: [u32; 4], pixel: [u32; 4]) -> CommandPlan {
    let mut values = vec![
        (RegisterSpace::Sh, 0x8b, 0x0003_0020),
        (RegisterSpace::Sh, 0x0b, 0x20),
    ];
    for (first, descriptor) in [(0x8c, vertex), (0x0c, pixel)] {
        let base = if first == 0x8c {
            vertex_base(base)
        } else {
            base
        };
        values.extend(
            descriptor
                .into_iter()
                .enumerate()
                .map(|(lane, value)| (RegisterSpace::Sh, first + base + lane as u32, value)),
        );
    }
    plan(&commands(&values))
}

#[test]
fn actual_raw_and_scalar_graphics_resources_keep_independent_stage_bindings_and_snapshots() {
    for base in [0, 4, 12] {
        for scalar in [false, true] {
            let memory = Buffers::new(base, scalar);
            let planned = draw(
                base,
                descriptor(VERTEX_BUFFER, 16),
                descriptor(PIXEL_BUFFER, 16),
            );
            let prepared = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
            for (resource, shader, push, address, first) in [
                (
                    prepared.vertex_buffer().unwrap(),
                    prepared.vertex(),
                    prepared.vertex_push_data(),
                    VERTEX_BUFFER,
                    0,
                ),
                (
                    prepared.pixel_buffer().unwrap(),
                    prepared.pixel(),
                    prepared.pixel_push_data(),
                    PIXEL_BUFFER,
                    16,
                ),
            ] {
                assert_eq!(
                    resource.range(),
                    crate::compute::CommandRange { address, size: 16 }
                );
                assert_eq!(
                    shader.manifest().buffer_binding(),
                    Some(if first == 0 { 0 } else { 57 })
                );
                assert_eq!(
                    shader.manifest().buffers()[0].user_sgpr,
                    (if first == 0 { vertex_base(base) } else { base }) as u8
                );
                assert_eq!(shader.manifest().buffers()[0].scalar, scalar);
                let layout = shader.manifest().push_data().unwrap();
                assert_eq!(layout.memory_offset_count, 1);
                assert_eq!(layout.start_dword, first);
                assert_eq!(
                    &push[first as usize * 4..first as usize * 4 + 16],
                    &bytes(&descriptor(address, 16))
                );
                let at = (layout.start_dword + layout.memory_offset_dword) as usize * 4;
                assert_eq!(&push[at..at + 4], &[0; 4]);
            }
            assert_eq!(
                memory.reads.borrow().as_slice(),
                [(VERTEX_BUFFER, 16), (PIXEL_BUFFER, 16)]
            );
            let first = prepared.pixel_buffer().unwrap().bytes().to_vec();
            memory.ranges[1].1.borrow_mut()[..4].copy_from_slice(&0.75_f32.to_bits().to_le_bytes());
            assert_eq!(prepared.pixel_buffer().unwrap().bytes(), first);
            let second = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
            assert_ne!(second.pixel_buffer().unwrap().bytes(), first);
            assert_eq!(
                prepared.usage().host_bytes,
                prepared.input.capacity()
                    + prepared.vertex().retained_host_bytes()
                    + prepared.pixel().retained_host_bytes()
                    + prepared.vertex_buffer.as_ref().unwrap().bytes.capacity()
                    + prepared.pixel_buffer.as_ref().unwrap().bytes.capacity()
            );
            assert_eq!(prepared.usage().pixel_bytes, prepared.input.capacity());
        }
    }
}

#[test]
fn resource_base_high_bits_short_ranges_and_read_aliases_keep_actual_descriptor_semantics() {
    let mut memory = Buffers::new(0, false);
    let high = (1_u64 << 32) | VERTEX_BUFFER;
    memory.ranges[0].0 = high;
    let planned = draw(0, descriptor(high, 4), descriptor(high, 4));
    let prepared = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
    assert_eq!(prepared.vertex_buffer().unwrap().range().address, high);
    assert_eq!(prepared.vertex_buffer().unwrap().bytes().len(), 4);
    assert_eq!(
        prepared.pixel_buffer().unwrap().bytes(),
        prepared.vertex_buffer().unwrap().bytes()
    );
    assert_eq!(memory.reads.borrow().as_slice(), [(high, 4), (high, 4)]);
    // The original x4 read exceeds the descriptor; its compiled per-dword
    // checks, rather than a fabricated larger host binding, handle the tail.
    assert_eq!(
        prepared.vertex().manifest().buffers()[0].max_byte_extent,
        16
    );
}

#[test]
fn unknown_descriptor_fields_null_alignment_and_complete_extents_refuse_before_snapshots() {
    for invalid in [
        [0, 0, 16, 0],
        [VERTEX_BUFFER as u32, 0, 0, 0],
        [VERTEX_BUFFER as u32 + 1, 0, 16, 0],
        [VERTEX_BUFFER as u32, 0, 15, 0],
        [VERTEX_BUFFER as u32, 1 << 16, 16, 0],
        [VERTEX_BUFFER as u32, 0, 16, 1],
        [0, 0x8000, 16, 0],
        [0xffff_fffc, 0x7fff, 8, 0],
    ] {
        let memory = Buffers::new(0, false);
        let planned = draw(0, invalid, descriptor(PIXEL_BUFFER, 16));
        assert!(matches!(
            prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
            Err(Error::Unsupported(_))
        ));
        assert!(memory.reads.borrow().is_empty());
        assert_eq!(memory.source.snapshots.get(), 0);
    }
}

#[test]
fn raw_resource_permissions_holes_virtual_target_aliases_and_byte_quotas_never_make_a_draw() {
    let memory = Buffers::new(0, false);
    memory.refused.set(true);
    let planned = draw(
        0,
        descriptor(VERTEX_BUFFER, 16),
        descriptor(PIXEL_BUFFER, 16),
    );
    assert!(matches!(
        prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
        Err(Error::Memory(MemoryError::ReadDenied))
    ));
    assert_eq!(memory.source.snapshots.get(), 0);
    let memory = Buffers::new(0, false);
    for (address, size, expected) in [
        (0x50000, 16, "hole"),
        (TARGET, 16, "alias"),
        (VERTEX_BUFFER, 0xffff_fffc, "cap"),
    ] {
        let planned = draw(0, descriptor(address, size), descriptor(PIXEL_BUFFER, 16));
        let result = prepare_draw(&planned.draws()[0], &memory, &Limits::default());
        match expected {
            "hole" => assert!(matches!(
                result,
                Err(Error::Memory(MemoryError::Unavailable))
            )),
            "alias" => assert!(matches!(
                result,
                Err(Error::Unsupported("render target overlaps buffer source"))
            )),
            _ => assert!(matches!(result, Err(Error::Limit(_)))),
        }
        assert_eq!(memory.source.snapshots.get(), 0);
    }
}

#[test]
fn stores_coherent_dynamic_multiple_and_overlapping_push_profiles_remain_explicit_refusals() {
    for kind in [
        "store",
        "coherent",
        "dynamic",
        "overwritten",
        "multiple",
        "push",
    ] {
        let mut memory = Buffers::new(0, false);
        match kind {
            "store" => {
                let mut source = vec![
                    unary(1, 6, 128),
                    unary(1, 7, 128),
                    0xe000_0000 | (0x1c << 18) | (1 << 12),
                    (128 << 24) | (2 << 16) | (6 << 8) | 7,
                ];
                source.extend(program(true, 8, false));
                memory.source.vertex = source;
            }
            "coherent" => memory.source.vertex[0] |= 1 << 14,
            "dynamic" => memory
                .source
                .vertex
                .insert(0, 0xbe80_0000 | (8 << 16) | (3 << 8) | 128),
            "overwritten" => memory.source.vertex = program(true, 0, false),
            "multiple" => {
                let mut source = vec![
                    0xe000_0000 | (0x0e << 18),
                    (128 << 24) | (1 << 16) | (4 << 8),
                ];
                source.extend([memory.source.vertex[0], memory.source.vertex[1]]);
                source.extend(export(12, [0, 4, 2, 3]));
                source.push(END);
                memory.source.vertex = source;
            }
            _ => {
                let mut source = vec![
                    memory.source.pixel[0],
                    memory.source.pixel[1],
                    0x8000_0000 | (40 << 16) | (5 << 8) | 4,
                ];
                for register in 6..16 {
                    source.push(0x8000_0000 | (40 << 16) | (register << 8) | 40);
                }
                source.push(unary(1, 0, 40));
                source.extend(export(0, [0, 1, 2, 3]));
                source.push(END);
                memory.source.pixel = source;
            }
        }
        let planned = draw(
            0,
            descriptor(VERTEX_BUFFER, 16),
            descriptor(PIXEL_BUFFER, 16),
        );
        assert!(
            matches!(
                prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
                Err(Error::Compile | Error::Unsupported(_))
            ),
            "{kind}"
        );
        assert!(memory.reads.borrow().is_empty(), "{kind}");
        assert_eq!(memory.source.snapshots.get(), 0);
    }
}

#[test]
fn no_resource_draws_and_dead_raw_loads_keep_the_existing_owned_contract() {
    let mut memory = Source::new();
    let mut vertex = vec![0xe000_0000 | (0x0e << 18), (128 << 24) | (8 << 8)];
    vertex.append(&mut memory.vertex);
    memory.vertex = vertex;
    let planned = plan(&commands(&[]));
    let prepared = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
    assert!(prepared.vertex_buffer().is_none() && prepared.pixel_buffer().is_none());
    assert!(prepared.vertex().manifest().buffers().is_empty());
    assert_eq!(memory.snapshots.get(), 1);
}
