// SPDX-License-Identifier: GPL-2.0-only
//! Original index packets and resources, with no native or GPU execution.
use super::*;

const INDEX: u64 = 0x18000;

fn indexed(address: u64, kind: u32, maximum: u32, count: u32, flags: u32) -> Vec<u32> {
    let mut words = commands(&[]);
    words.truncate(words.len() - 3);
    set(&mut words, RegisterSpace::Uconfig, 0x24b, &[0]);
    words.extend([
        0xc000_2a00,
        kind,
        0xc004_2700,
        maximum,
        address as u32,
        (address >> 32) as u32,
        count,
        flags,
    ]);
    words
}

struct Indices {
    source: Source,
    bytes: RefCell<Vec<u8>>,
    denied: Cell<bool>,
    captures: Cell<usize>,
}
impl Indices {
    fn new(kind: IndexType) -> Self {
        Self {
            source: Source::new(),
            bytes: RefCell::new(match kind {
                IndexType::U16 => [0_u16, 1, 3]
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                IndexType::U32 => [0_u32, 1, 3]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect(),
            }),
            denied: Cell::new(false),
            captures: Cell::new(0),
        }
    }
}
impl Memory for Indices {
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
        if address == INDEX {
            assert!(read && !written);
            self.captures.set(self.captures.get() + 1);
            if self.denied.get() {
                return Err(MemoryError::ReadDenied);
            }
            let bytes = self.bytes.borrow();
            let source = bytes.get(..output.len()).ok_or(MemoryError::Unavailable)?;
            output.copy_from_slice(source);
            Ok(())
        } else if read && !written {
            for (index, chunk) in output.chunks_mut(4).enumerate() {
                let mut word = [0; 4];
                self.source.read(address + index as u64 * 4, &mut word)?;
                chunk.copy_from_slice(&word[..chunk.len()]);
            }
            Ok(())
        } else {
            self.source.snapshot_buffer(address, output, read, written)
        }
    }
}

#[test]
fn indexed_packets_keep_exact_types_count_bounds_state_and_zero_count_noop() {
    for kind in [0, 1] {
        let planned = plan(&indexed(INDEX, kind, 3, 3, 0));
        assert_eq!(planned.draws().len(), 1);
        let source = planned.draws()[0].indices.unwrap();
        assert_eq!(source.address, INDEX);
        assert_eq!(
            source.index_type,
            if kind == 0 {
                IndexType::U16
            } else {
                IndexType::U32
            }
        );
        assert_eq!(planned.usage().vertices, 3);
    }
    // No payload or unknown index-type state is needed for an admitted no-op.
    let zero = plan(&[0xc000_2f00, 1, 0xc004_2700, 0, 0, 0, 0, 0]);
    assert!(zero.draws().is_empty());
    assert_eq!(zero.usage().zero_draws, 1);
    for words in [
        indexed(INDEX, 2, 3, 3, 0),
        indexed(INDEX, 0, 2, 3, 0),
        indexed(INDEX, 0, 3, 3, 0x20),
        indexed(INDEX + 1, 0, 3, 3, 0),
        indexed((1 << 47) - 2, 0, 3, 3, 0),
        indexed(1 << 47, 0, 3, 3, 0),
        indexed(0, 0, 3, 3, 0),
        vec![0xc000_2f00, 1, 0xc004_2701, 3, INDEX as u32, 0, 3, 0],
        vec![0xc000_2f00, 1, 0xc003_2700, 3, INDEX as u32, 0, 3],
    ] {
        assert!(
            plan_commands(
                &[&bytes(&words)],
                GraphicsState::empty(),
                &CommandLimits::default()
            )
            .is_err()
        );
    }
    let missing_type = bytes(&[0xc000_2f00, 1, 0xc004_2700, 3, INDEX as u32, 0, 3, 0]);
    assert!(matches!(
        plan_commands(
            &[&missing_type],
            GraphicsState::empty(),
            &CommandLimits::default()
        ),
        Err(Error::Unsupported("unknown INDEX_TYPE"))
    ));
}

#[test]
fn indexed_preparation_captures_full_read_only_payload_and_all_actual_capacities() {
    for (encoded, kind) in [(0, IndexType::U16), (1, IndexType::U32)] {
        let memory = Indices::new(kind);
        let planned = plan(&indexed(INDEX, encoded, 3, 3, 0));
        let first = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
        assert_eq!(memory.captures.get(), 1);
        let payload = first.indices().unwrap();
        assert_eq!(payload.count(), 3);
        assert_eq!(payload.index_type(), kind);
        assert_eq!(payload.bytes(), memory.bytes.borrow().as_slice());
        assert_eq!(payload.range().address, INDEX);
        assert_eq!(payload.range().size, (3 * kind.byte_width()) as u64);
        assert_eq!(
            first.usage().host_bytes,
            first.vertex.retained_host_bytes()
                + first.pixel.retained_host_bytes()
                + first.input.capacity()
                + first.indices.as_ref().unwrap().bytes.capacity()
        );
        let snapshot = payload.bytes().to_vec();
        memory.bytes.borrow_mut().fill(0);
        let second = prepare_draw(&planned.draws()[0], &memory, &Limits::default()).unwrap();
        assert_eq!(first.indices().unwrap().bytes(), snapshot);
        assert!(
            second
                .indices()
                .unwrap()
                .bytes()
                .iter()
                .all(|byte| *byte == 0)
        );
        assert_eq!(memory.captures.get(), 2);
        memory.denied.set(true);
        assert!(matches!(
            prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
            Err(Error::Memory(MemoryError::ReadDenied))
        ));
        memory.denied.set(false);
        memory.bytes.borrow_mut().pop();
        assert!(matches!(
            prepare_draw(&planned.draws()[0], &memory, &Limits::default()),
            Err(Error::Memory(MemoryError::Unavailable))
        ));
    }
}

#[test]
fn indexed_source_alias_restart_and_resource_refusals_leave_inputs_unchanged() {
    let memory = Indices::new(IndexType::U16);
    let original = memory.source.target.clone();
    let overlap = plan(&indexed(TARGET, 0, 3, 3, 0));
    assert!(matches!(
        prepare_draw(&overlap.draws()[0], &memory, &Limits::default()),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(memory.captures.get(), 0);
    // Index and shader bytes are read-only inputs and may legitimately share
    // current backing. Only the write target is excluded from their read set.
    let shared = plan(&indexed(VS, 0, 3, 3, 0));
    let captured = prepare_draw(&shared.draws()[0], &memory, &Limits::default()).unwrap();
    assert_eq!(
        captured.indices().unwrap().bytes(),
        &bytes(&memory.source.vertex)[..6]
    );
    let mut words = indexed(INDEX, 0, 3, 3, 0);
    // Override explicit restart state immediately before the indexed packet.
    let split = words.len() - 6;
    let tail = words.split_off(split);
    set(&mut words, RegisterSpace::Uconfig, 0x24b, &[1]);
    words.extend(tail);
    assert!(matches!(
        prepare_draw(&plan(&words).draws()[0], &memory, &Limits::default()),
        Err(Error::Unsupported("indexed primitive restart"))
    ));
    let planned = plan(&indexed(INDEX, 0, 3, 3, 0));
    for limits in [
        Limits {
            max_host_bytes: 1,
            ..Default::default()
        },
        Limits {
            max_work: 1,
            ..Default::default()
        },
        Limits {
            max_vertices: 2,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            prepare_draw(&planned.draws()[0], &memory, &limits),
            Err(Error::Limit(_))
        ));
    }
    assert_eq!(memory.source.target, original);
}
