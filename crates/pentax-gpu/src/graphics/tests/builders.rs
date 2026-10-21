// SPDX-License-Identifier: GPL-2.0-only
//! Original source-builder packet shapes; no device or GPU execution.
use super::*;

const INDEX: u64 = 0x18000;

fn offset(kind: u32, cache: u32, base: u64, offset: u32, count: u32) -> Vec<u32> {
    let mut words = commands(&[]);
    words.truncate(words.len() - 3);
    set(&mut words, RegisterSpace::Uconfig, 0x24b, &[0]);
    words.extend([
        0xc001_7a00,
        0x2000_0243,
        0x400 | kind | (cache << 6),
        0xc001_2600,
        base as u32,
        (base >> 32) as u32,
        0xc000_1300,
        1, // Retained state, not a made-up direct-draw truncation.
        0xc003_3500,
        count.max(1),
        offset,
        count,
        0,
    ]);
    words
}

#[test]
fn builder_index_state_and_offset_draws_use_exact_count_width_and_checked_address() {
    for (kind, width) in [(0, 2), (1, 4)] {
        for cache in 0..4 {
            let words = offset(kind, cache, INDEX - 2 * width, 2, 3);
            let first = plan(&words);
            let source = first.draws()[0].indices.unwrap();
            assert_eq!(source.address, INDEX);
            assert_eq!(source.index_type.byte_width() as u64, width);
            assert_eq!(first.draws()[0].vertex_count(), 3);
            assert_eq!(first.final_state().index_count, Some(1));
            assert_eq!(first.usage().vertices, 3);
            let next = bytes(&[0xc003_3500, 3, 3, 3, 0]);
            let second = plan_commands(&[&next], first.final_state(), &Default::default()).unwrap();
            assert_eq!(second.draws()[0].indices.unwrap().address, INDEX + width);
        }
    }
    let zero = plan(&[0xc000_2f00, 1, 0xc003_3500, 1, u32::MAX, 0, 0]);
    assert!(zero.draws().is_empty());
    assert_eq!(zero.usage().zero_draws, 1);
    for words in [
        offset(2, 0, INDEX, 0, 3),
        offset(0, 0, INDEX + 1, 0, 3),
        offset(1, 0, (1 << 47) - 4, 2, 3),
        vec![0xc001_7a00, 0x1000_0243, 0x400],
        vec![0xc001_7a00, 0x2000_0243, 0x500],
        vec![0xc001_2600, 0, 0x8000],
        vec![0xc000_2f00, 1, 0xc003_3500, 3, 0, 3, 0x20],
        vec![0xc000_2f00, 1, 0xc003_3500, 2, 0, 3, 0],
        vec![0xc000_2f00, 1, 0xc001_2500, 0, 0],
    ] {
        assert!(
            plan_commands(
                &[&bytes(&words)],
                GraphicsState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
}

struct Indexed {
    source: Source,
    bytes: RefCell<Vec<u8>>,
    captures: RefCell<Vec<(u64, usize)>>,
    denied: Cell<bool>,
}
impl Memory for Indexed {
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
            self.captures.borrow_mut().push((address, output.len()));
            if self.denied.get() {
                return Err(MemoryError::ReadDenied);
            }
            let bytes = self.bytes.borrow();
            output.copy_from_slice(bytes.get(..output.len()).ok_or(MemoryError::Unavailable)?);
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
fn offset_index_preparation_captures_exact_live_read_tuple_and_refuses_current_denial() {
    for (kind, width) in [(0, 2), (1, 4)] {
        let payload: Vec<u8> = if kind == 0 {
            [0u16, 1, 3]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect()
        } else {
            [0u32, 1, 3]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect()
        };
        let memory = Indexed {
            source: Source::new(),
            bytes: RefCell::new(payload.clone()),
            captures: RefCell::new(Vec::new()),
            denied: Cell::new(false),
        };
        let planned = plan(&offset(kind, 3, INDEX - 2 * width, 2, 3));
        let prepared = prepare_draw(&planned.draws()[0], &memory, &Default::default()).unwrap();
        assert_eq!(prepared.indices().unwrap().bytes(), payload);
        assert_eq!(*memory.captures.borrow(), [(INDEX, 3 * width as usize)]);
        memory.bytes.borrow_mut().fill(0);
        let current = prepare_draw(&planned.draws()[0], &memory, &Default::default()).unwrap();
        assert!(
            current
                .indices()
                .unwrap()
                .bytes()
                .iter()
                .all(|byte| *byte == 0)
        );
        assert_eq!(prepared.indices().unwrap().bytes(), payload);
        memory.denied.set(true);
        assert!(matches!(
            prepare_draw(&planned.draws()[0], &memory, &Default::default()),
            Err(Error::Memory(MemoryError::ReadDenied))
        ));
    }
}

#[test]
fn graphics_memory_actions_remain_ordered_owned_bounded_and_break_old_flip_authority() {
    for data in 1..=3 {
        for interrupt in [1, 4] {
            for (destination, address) in [(0, 3), (1, 0)] {
                // Graphics INT1 and every INT4 drop the label before address
                // and event validation. The raw invalid label is never used.
                let raw = [
                    0xc006_1060,
                    0x600,
                    (data << 29) | (interrupt << 24) | (destination << 16),
                    address,
                    0,
                    0x1234_5678,
                    0x89ab_cdef,
                    0xf123_4567,
                ];
                let planned = plan(&raw);
                let MemoryAction::Release(release) = planned.memory_action(0) else {
                    panic!("release")
                };
                assert!(release.write().is_none());
                assert!(!release.reference_clock());
                assert_eq!(release.interrupt_context(), Some(0x0123_4567));
            }
        }
    }
    for (size, central) in [(4, 0xc005_3c00), (8, 0xc007_9300)] {
        let mut tagged = vec![
            0xc002_7904,
            0x342,
            if size == 4 { 0xc801_0000 } else { 0xc802_0000 },
            0x3003,
            central,
            0x13 | (3 << 25),
            0x3000,
            0,
            17,
        ];
        if size == 8 {
            tagged.push(1);
        }
        tagged.push(u32::MAX);
        if size == 8 {
            tagged.push(u32::MAX);
        }
        tagged.extend([0, 0xc001_7904, 0x342, 0xc800_0000]);
        let planned = plan(&tagged);
        assert_eq!(planned.usage().packets, 3);
        assert_eq!(planned.actions(), &[Action::Memory(0)]);
        let MemoryAction::Wait(wait) = planned.memory_action(0) else {
            panic!("tagged wait")
        };
        assert_eq!(wait.byte_count(), size);
        assert!(matches!(
            plan_commands(
                &[&bytes(&tagged)],
                GraphicsState::empty(),
                &CommandLimits {
                    max_packets: 2,
                    ..Default::default()
                }
            ),
            Err(Error::Limit(_))
        ));
    }
    let write = [0xc004_3700, 0x4000_0200 | (3 << 25), 0x30000, 0, 17, 29];
    let release = [0xc006_1060, 0x62f, 1 << 29, 0x30008, 0, 31, 0, 0];
    let wait = [0xc005_3c00, 0x13 | (2 << 25), 0x30000, 0, 17, u32::MAX, 0];
    let mut words = write.to_vec();
    words.extend(commands(&[]));
    words.extend(release);
    words.extend(wait);
    words.extend([0xc000_4200, 0]);
    words.extend([0xc001_2d00, 3, 2]);
    let planned = plan(&words);
    assert_eq!(
        planned.actions(),
        &[
            Action::Memory(0),
            Action::Draw(0),
            Action::Memory(1),
            Action::Memory(2),
            Action::Memory(3),
            Action::Draw(1)
        ]
    );
    let MemoryAction::Write(captured) = planned.memory_action(0) else {
        panic!("write")
    };
    assert_eq!(captured.address(), 0x30000);
    assert_eq!(captured.bytes(), bytes(&[17, 29]));
    let MemoryAction::Release(captured) = planned.memory_action(1) else {
        panic!("release")
    };
    assert_eq!(captured.write().unwrap().bytes(), 31u32.to_le_bytes());
    assert!(matches!(planned.memory_action(2), MemoryAction::Wait(_)));
    assert!(matches!(planned.memory_action(3), MemoryAction::Drain));
    let baseline = words.clone();
    for limits in [
        CommandLimits {
            max_host_bytes: 1,
            ..Default::default()
        },
        CommandLimits {
            max_work: planned.usage().work - 1,
            ..Default::default()
        },
        CommandLimits {
            max_draws: 2,
            ..Default::default()
        },
    ] {
        assert!(plan_commands(&[&bytes(&words)], GraphicsState::empty(), &limits).is_err());
        assert_eq!(words, baseline);
    }
    let mut stale = commands(&[]);
    stale.extend(write);
    stale.extend([0xc004_105c, 1, 0, 0, 0, 0]);
    assert!(matches!(
        plan_commands(
            &[&bytes(&stale)],
            GraphicsState::empty(),
            &Default::default()
        ),
        Err(Error::Unsupported("GPU flip needs a fresh draw"))
    ));
    let mut repaired = stale[..stale.len() - 6].to_vec();
    repaired.extend([0xc001_2d00, 3, 2]);
    repaired.extend([0xc004_105c, 1, 0, 0, 0, 0]);
    assert_eq!(plan(&repaired).usage().flips, 1);
}
