// SPDX-License-Identifier: GPL-2.0-only
//! Original PM4/shader/data fixtures, never imported guest or upstream programs.

use super::*;
use std::cell::{Cell, RefCell};

mod flips;
mod indexed;
mod staging;
mod tiled;

const VS: u64 = 0x10000;
const PS: u64 = 0x11000;
const TARGET: u64 = 0x20000;
const END: u32 = 0xbf81_0000;

fn unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e00_0000 | (dst << 17) | (op << 9) | src
}
fn binary(op: u32, dst: u32, src: u32, vector: u32) -> u32 {
    (op << 25) | (dst << 17) | (vector << 9) | src
}
fn export(target: u32, registers: [u32; 4]) -> [u32; 2] {
    [
        0xf800_080f | (target << 4),
        registers[0] | (registers[1] << 8) | (registers[2] << 16) | (registers[3] << 24),
    ]
}
struct Source {
    vertex: Vec<u32>,
    pixel: Vec<u32>,
    target: Vec<u8>,
    reads: RefCell<Vec<(u64, usize)>>,
    snapshots: Cell<usize>,
    deny_write: Cell<bool>,
}
impl Source {
    fn new() -> Self {
        // An original fullscreen triangle from actual VertexIndex (v5):
        // x=2*((id<<1)&2)-1; y=2*(id&2)-1. No vertex-data buffer or host shader.
        let mut vertex = vec![
            binary(0x1a, 0, 129, 5),
            binary(0x1b, 0, 130, 0),
            binary(0x1b, 1, 130, 5),
            unary(6, 0, 256),
            unary(6, 1, 257),
            binary(8, 0, 244, 0),
            binary(8, 1, 244, 1),
            binary(3, 0, 243, 0),
            binary(3, 1, 243, 1),
            unary(1, 2, 128),
            unary(1, 3, 242),
        ];
        vertex.extend(export(12, [0, 1, 2, 3]));
        vertex.push(END);
        // Actual PS user SGPR0 supplies the red channel's raw f32 bits.
        let mut pixel = vec![
            unary(1, 0, 0),
            unary(1, 1, 128),
            unary(1, 2, 128),
            unary(1, 3, 242),
        ];
        pixel.extend(export(0, [0, 1, 2, 3]));
        pixel.push(END);
        Self {
            vertex,
            pixel,
            target: (0..64 * 64 * 4).map(|i| (i % 251) as u8).collect(),
            reads: RefCell::new(Vec::new()),
            snapshots: Cell::new(0),
            deny_write: Cell::new(false),
        }
    }
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.borrow_mut().push((address, output.len()));
        if address == TARGET && output.len() == self.target.len() {
            output.copy_from_slice(&self.target);
            return Ok(());
        }
        for (base, words) in [(VS, &self.vertex), (PS, &self.pixel)] {
            if let Some(offset) = address.checked_sub(base)
                && output.len() == 4
                && offset.is_multiple_of(4)
                && let Some(word) = usize::try_from(offset / 4).ok().and_then(|i| words.get(i))
            {
                output.copy_from_slice(&word.to_le_bytes());
                return Ok(());
            }
        }
        Err(MemoryError::Unavailable)
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), MemoryError> {
        assert!(read && written);
        self.snapshots.set(self.snapshots.get() + 1);
        if self.deny_write.get() {
            return Err(MemoryError::WriteDenied);
        }
        self.read(address, output)
    }
}
fn set(words: &mut Vec<u32>, space: RegisterSpace, register: u32, values: &[u32]) {
    words.push(0xc000_0000 | ((values.len() as u32) << 16) | (u32::from(space.opcode()) << 8));
    words.push(register);
    words.extend_from_slice(values);
}
fn commands(extra: &[(RegisterSpace, u32, u32)]) -> Vec<u32> {
    let mut words = Vec::new();
    for reg in [
        0x80, 0x1b1, 0x1b3, 0x1b4, 0x1b8, 0x1c4, 0x1e0, 0x200, 0x203, 0x205, 0x207, 0x293, 0x2f8,
        0x31b, 0x31d, 0x31e, 0x31f, 0x321, 0x325, 0x390, 0x398, 0x3a0, 0x3a8,
    ] {
        set(&mut words, RegisterSpace::Context, reg, &[0]);
    }
    for (reg, value) in [
        (0x8e, 15),
        (0x8f, 15),
        (0x1b6, 0x8000),
        (0x1c3, 4),
        (0x1c5, 9),
        (0x202, 0x00cc_0010),
        (0x204, 1 << 19),
        (0x206, 0x3f),
        (0x292, 2),
        (0x2d5, 0x0040_0000),
        (0x318, (TARGET >> 8) as u32),
        (0x31c, (10 << 2) | (1 << 15)),
        (0x3b0, (63 << 14) | 63),
        (0x3b8, 1 << 24),
    ] {
        set(&mut words, RegisterSpace::Context, reg, &[value]);
    }
    for (tl, br, left) in [
        (0x0c, 0x0d, 0),
        (0x81, 0x82, 0x8000_0000),
        (0x90, 0x91, 0x8000_0000),
        (0x94, 0x95, 0x8000_0000),
    ] {
        set(&mut words, RegisterSpace::Context, tl, &[left]);
        set(&mut words, RegisterSpace::Context, br, &[64 | (64 << 16)]);
    }
    set(
        &mut words,
        RegisterSpace::Context,
        0x10f,
        &[
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            1_f32.to_bits(),
            0,
        ],
    );
    set(&mut words, RegisterSpace::Sh, 0xc8, &[(VS >> 8) as u32, 0]);
    set(
        &mut words,
        RegisterSpace::Sh,
        0x88,
        &[0, 0, 0x602c_0000, 0x0003_0000],
    );
    set(
        &mut words,
        RegisterSpace::Sh,
        0x08,
        &[(PS >> 8) as u32, 0, 0x002c_0000, 2, 0.5_f32.to_bits()],
    );
    set(&mut words, RegisterSpace::Uconfig, 0x242, &[4]);
    set(&mut words, RegisterSpace::Uconfig, 0x24a, &[0]);
    words.extend([0xc000_2f00, 1]);
    for &(space, register, value) in extra {
        set(&mut words, space, register, &[value]);
    }
    words.extend([0xc001_2d00, 3, 2]);
    words
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn plan(words: &[u32]) -> CommandPlan {
    plan_commands(
        &[&bytes(words)],
        GraphicsState::empty(),
        &CommandLimits::default(),
    )
    .unwrap()
}

#[test]
fn real_draw_snapshots_compile_guest_shaders_and_preserve_target_and_stage_pushes() {
    let memory = Source::new();
    let mut words = commands(&[]);
    set(&mut words, RegisterSpace::Sh, 0x0c, &[0.25_f32.to_bits()]);
    words.extend([0xc000_2f00, 2, 0xc001_2d00, 6, 2]);
    let plan = plan(&words);
    assert_eq!(plan.draws().len(), 2);
    assert!(memory.reads.borrow().is_empty());
    let first = prepare_draw(&plan.draws()[0], &memory, &Limits::default()).unwrap();
    let second = prepare_draw(&plan.draws()[1], &memory, &Limits::default()).unwrap();
    assert_eq!(
        (first.width(), first.height(), first.target_address()),
        (64, 64, TARGET)
    );
    assert_eq!(
        (
            first.vertex_count(),
            first.instance_count(),
            first.first_vertex(),
            first.first_instance()
        ),
        (3, 1, 0, 0)
    );
    assert_eq!((second.vertex_count(), second.instance_count()), (6, 2));
    assert_eq!(first.input(), memory.target.as_slice());
    assert_eq!(second.input(), memory.target.as_slice());
    assert_eq!(first.vertex().spirv()[0], 0x0723_0203);
    assert_eq!(first.pixel().spirv()[0], 0x0723_0203);
    assert_eq!(
        first.vertex().manifest().outputs(),
        [pentax_shader_compiler::OutputInterface::Position]
    );
    let layout = first.pixel().manifest().push_data().unwrap();
    assert_eq!(
        (layout.start_dword, layout.user_sgprs.as_slice()),
        (16, &[0_u8][..])
    );
    assert_eq!(
        &first.pixel_push_data()[64..68],
        &0.5_f32.to_bits().to_le_bytes()
    );
    assert_eq!(
        &second.pixel_push_data()[64..68],
        &0.25_f32.to_bits().to_le_bytes()
    );
    assert_eq!(
        first.usage().shader_words,
        memory.vertex.len() + memory.pixel.len()
    );
    assert_eq!(
        first.shader_ranges(),
        [
            crate::compute::CommandRange {
                address: VS,
                size: (memory.vertex.len() * 4) as u64
            },
            crate::compute::CommandRange {
                address: PS,
                size: (memory.pixel.len() * 4) as u64
            },
        ]
    );
    assert_eq!(plan.usage().vertices, 15);
    assert_eq!(memory.snapshots.get(), 2);
}

#[test]
fn complete_admission_and_known_fixed_state_refuse_before_any_resource_read() {
    let memory = Source::new();
    let initial = GraphicsState::empty();
    let mut bad = commands(&[]);
    bad.extend([0xc000_ff00, 0]);
    assert!(matches!(
        plan_commands(&[&bytes(&bad)], initial, &CommandLimits::default()),
        Err(Error::Packet { .. })
    ));
    assert_eq!(initial, GraphicsState::empty());
    let missing = plan(&[0xc000_2f00, 1, 0xc001_2d00, 3, 2]);
    assert!(matches!(
        prepare_draw(&missing.draws()[0], &memory, &Limits::default()),
        Err(Error::MissingRegister { .. })
    ));
    for (register, value) in [
        (0x200, 2),
        (0x31c, (10 << 2) | (1 << 28)),
        (0x3b8, (1 << 24) | (9 << 14)),
        (0x3b8, 2 << 24),
        (0x3b0, (62 << 14) | 63),
        (0x204, (1 << 19) | (1 << 16)),
        (0x10f, f32::NAN.to_bits()),
    ] {
        let plan = plan(&commands(&[(RegisterSpace::Context, register, value)]));
        assert!(matches!(
            prepare_draw(&plan.draws()[0], &memory, &Limits::default()),
            Err(Error::Unsupported(_))
        ));
    }
    let normal = plan(&commands(&[]));
    assert!(matches!(
        prepare_draw(
            &normal.draws()[0],
            &memory,
            &Limits {
                max_host_bytes: 0,
                ..Default::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(memory.snapshots.get(), 0);
}

#[test]
fn genuine_shader_boundaries_backedges_and_gpu_write_refusal_never_return_replacements() {
    let mut memory = Source::new();
    let planned = plan(&commands(&[]));
    memory.vertex.splice(..0, [0xbe8a_03ff, END]); // S_MOV_B32 literal is not a terminator.
    let first = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
    assert_eq!(
        first.usage().shader_words,
        memory.vertex.len() + memory.pixel.len()
    );
    memory.pixel[0] = unary(1, 0, 242); // Different real shader, not a title/pattern shortcut.
    let changed = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
    assert_ne!(first.pixel().spirv(), changed.pixel().spirv());
    memory.deny_write.set(true);
    assert!(matches!(
        prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
        Err(Error::Memory(MemoryError::WriteDenied))
    ));
    let snapshots = memory.snapshots.get();
    let overlapping = plan(&commands(&[(
        RegisterSpace::Context,
        0x318,
        (VS >> 8) as u32,
    )]));
    assert!(matches!(
        prepare_draw(&overlapping.draws()[0], &memory, &Limits::default()),
        Err(Error::Unsupported("render target overlaps shader source"))
    ));
    assert_eq!(memory.snapshots.get(), snapshots);
    memory.vertex = vec![0xbf82_ffff, END];
    assert!(matches!(
        prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
        Err(Error::Unsupported("unbounded graphics shader loop"))
    ));
    assert_eq!(memory.snapshots.get(), snapshots);
    assert_eq!(memory.target.as_slice(), first.input());
}

#[test]
fn command_bounds_state_only_zero_draws_and_unsupported_initiators_are_explicit() {
    let zero = plan(&[0xc000_2f00, 0, 0xc001_2d00, u32::MAX, 2]);
    assert!(zero.draws().is_empty());
    assert_eq!(zero.usage().zero_draws, 1);
    let setup = bytes(&[0xc000_2f00, 1]);
    let draw = bytes(&[0xc001_2d00, 3, 2]);
    let first =
        plan_commands(&[&setup], GraphicsState::empty(), &CommandLimits::default()).unwrap();
    assert!(first.draws().is_empty());
    assert_eq!(
        plan_commands(&[&draw], first.final_state(), &CommandLimits::default())
            .unwrap()
            .draws()
            .len(),
        1
    );
    for flags in [0, 0x20, 0x22] {
        assert!(matches!(
            plan_commands(
                &[&bytes(&[0xc000_2f00, 1, 0xc001_2d00, 3, flags])],
                GraphicsState::empty(),
                &CommandLimits::default()
            ),
            Err(Error::Packet { .. })
        ));
    }
    assert!(matches!(
        plan_commands(&[&draw], GraphicsState::empty(), &CommandLimits::default()),
        Err(Error::Unsupported("unknown NUM_INSTANCES"))
    ));
    for limits in [
        CommandLimits {
            max_host_bytes: 0,
            ..Default::default()
        },
        CommandLimits {
            max_work: 0,
            ..Default::default()
        },
        CommandLimits {
            max_packets: 0,
            ..Default::default()
        },
        CommandLimits {
            max_command_bytes: 0,
            ..Default::default()
        },
        CommandLimits {
            max_draws: 0,
            ..Default::default()
        },
        CommandLimits {
            max_vertices: 2,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            plan_commands(&[&setup, &draw], GraphicsState::empty(), &limits),
            Err(Error::Limit(_))
        ));
    }
}
