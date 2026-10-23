// SPDX-License-Identifier: GPL-2.0-only
//! Original sealed draw/compiler/backend instance-contract consumers.
use super::*;
use pentax_shader_compiler::{InputInterface, VertexInstanceContract};
struct NativeSource {
    source: Source,
}
impl NativeSource {
    fn new(relative: bool) -> Self {
        let mut source = Source::new();
        if relative {
            let at = source.vertex.len() - 3;
            source.vertex.splice(
                at..at,
                [
                    unary(6, 6, 264),
                    unary(6, 7, 0),
                    binary(3, 6, 263, 6),
                    binary(3, 0, 262, 0),
                ],
            );
        }
        Self { source }
    }
}
impl Memory for NativeSource {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        if address == 0x8000 && output.len() == 20 {
            output.copy_from_slice(&bytes(&[3, 2, 0, 0, 5]));
            return Ok(());
        }
        if address == 0x9000 && output.len() == 6 {
            output.copy_from_slice(&[0, 0, 1, 0, 2, 0]);
            return Ok(());
        }
        self.source.read(address, output)
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), MemoryError> {
        if address == 0x9000 {
            assert!(read && !written);
            return self.read(address, output);
        }
        self.source.snapshot_buffer(address, output, read, written)
    }
}
fn native_draw(memory: &NativeSource) -> Draw {
    let mut words = commands(&[(RegisterSpace::Sh, 0x8b, 0x0003_0002)]);
    words.truncate(words.len() - 3);
    words.extend([
        0xc000_2a00,
        0,
        0xc001_2600,
        0x9000,
        0,
        0xc000_1300,
        3,
        0xc002_1100,
        1,
        0x8000,
        0,
        0xc003_2500,
        0,
        0x280,
        0x8c,
        0,
    ]);
    let data = bytes(&words);
    let mut live = OrderedCommands::prepare(
        &[&data],
        &[CommandRange {
            address: 0x4000,
            size: data.len() as u64,
        }],
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        live.next_step().unwrap(),
        Some(OrderedStep::IndirectDraw)
    ));
    live.resolve_indirect(memory).unwrap().unwrap()
}
#[test]
fn sealed_native_draw_compiles_live_v8_without_base_instance_and_keeps_logical_sgpr_input() {
    let memory = NativeSource::new(true);
    let draw = native_draw(&memory);
    let prepared = prepare_draw(&draw, &memory, &Default::default()).unwrap();
    assert_eq!(
        prepared.vertex().manifest().vertex_instance_contract(),
        VertexInstanceContract::NativeIndirectFirstInstanceZero
    );
    assert_eq!(
        prepared.pixel().manifest().vertex_instance_contract(),
        VertexInstanceContract::Dynamic
    );
    assert!(
        prepared
            .vertex()
            .manifest()
            .inputs()
            .contains(&InputInterface::InstanceIndex)
    );
    assert!(
        !prepared
            .vertex()
            .manifest()
            .inputs()
            .contains(&InputInterface::BaseInstance)
    );
    assert!(
        !prepared
            .vertex()
            .manifest()
            .required_features()
            .draw_parameters()
    );
    assert_eq!(prepared.first_instance(), 0);
    assert_eq!(
        prepared.native_indirect_offsets().unwrap().first_instance(),
        5
    );
    assert_eq!(prepared.validate_native_instance_contract(), Ok(()));
    let layout = prepared.vertex().manifest().push_data().unwrap();
    let index = layout
        .user_sgprs
        .iter()
        .position(|register| *register == 0)
        .unwrap();
    assert_eq!(
        &prepared.vertex_push_data()[index * 4..index * 4 + 4],
        &5u32.to_le_bytes()
    );
    // The exact same live-v8 shader on the ordinary/direct path retains the
    // original actual feature requirement and its existing graphics refusal.
    let direct = plan(&commands(&[
        (RegisterSpace::Sh, 0x8b, 0x0003_0002),
        (RegisterSpace::Sh, 0x8c, 5),
    ]));
    assert!(matches!(
        prepare_draw(&direct.draws()[0], &memory, &Default::default()),
        Err(Error::Unsupported("graphics resource/subgroup manifest"))
    ));
}
#[test]
fn prepared_contract_rejects_both_compiler_and_sealed_mode_mismatch_directions() {
    let native_memory = NativeSource::new(false);
    let mut native = prepare_draw(
        &native_draw(&native_memory),
        &native_memory,
        &Default::default(),
    )
    .unwrap();
    let generic_memory = Source::new();
    let direct = plan(&commands(&[]));
    let mut generic =
        prepare_draw(&direct.draws()[0], &generic_memory, &Default::default()).unwrap();
    assert_eq!(generic.validate_native_instance_contract(), Ok(()));
    assert_eq!(native.validate_native_instance_contract(), Ok(()));
    // Only this in-crate original test can exchange private compiler products.
    // The backend calls this exact validation method before any device access.
    std::mem::swap(&mut native.vertex, &mut generic.vertex);
    assert_eq!(
        native.validate_native_instance_contract(),
        Err(Error::Unsupported("native indirect instance contract"))
    );
    assert_eq!(
        generic.validate_native_instance_contract(),
        Err(Error::Unsupported("native indirect instance contract"))
    );
}
