// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use std::cell::Cell;
mod builders;
const CODE: u64 = 0x10000;
const DATA: u64 = 0x20000;
struct Source {
    words: Vec<u32>,
    reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.set(self.reads.get() + 1);
        if address == DATA {
            output.fill(0xa5);
            return Ok(());
        }
        let index = address.checked_sub(CODE).ok_or(MemoryError::Unavailable)? as usize / 4;
        if output.len() != 4 {
            return Err(MemoryError::Unavailable);
        }
        output.copy_from_slice(
            &self
                .words
                .get(index)
                .ok_or(MemoryError::Unavailable)?
                .to_le_bytes(),
        );
        Ok(())
    }
}
fn source() -> Source {
    Source {
        words: vec![
            (0x25 << 25) | (1 << 17) | 4,
            (0x1a << 25) | (2 << 17) | 130,
            0xe070_1000,
            (128 << 24) | (1 << 8) | 2,
            0xbf81_0000,
        ],
        reads: Cell::new(0),
    }
}
fn commands() -> Vec<u8> {
    let mut words = Vec::new();
    let mut set = |first: u32, values: &[u32]| {
        words.push(0xc000_7602 | (values.len() as u32) << 16);
        words.push(first);
        words.extend(values);
    };
    set(0x207, &[16, 1, 1]);
    set(0x20c, &[(CODE >> 8) as u32, 0]);
    set(0x212, &[0x2c_0000, 10]);
    set(0x228, &[0]);
    set(0x240, &[DATA as u32, 0, 64, 0, 17]);
    words.extend([0xc003_1502, 1, 1, 1, 0x8005]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn prepared_compute_uses_actual_registers_shader_and_buffer_without_mutating_source() {
    let memory = source();
    let prepared = prepare(&commands(), &memory, &Limits::default()).unwrap();
    assert_eq!(prepared.groups(), [1, 1, 1]);
    assert_eq!(prepared.usage().invocations, 16);
    assert_eq!(
        prepared.shader().manifest().workgroup_size(),
        Some([16, 1, 1])
    );
    assert_eq!(prepared.buffer_address(), Some(DATA));
    assert_eq!(prepared.input().unwrap(), &[0xa5; 64]);
    let layout = prepared.shader().manifest().push_data().unwrap();
    let seed = layout.user_sgprs.iter().position(|r| *r == 4).unwrap();
    let at = (layout.start_dword as usize + seed) * 4;
    assert_eq!(&prepared.push_data()[at..at + 4], &17_u32.to_le_bytes());
    assert!(prepared.writes_buffer());
}
#[test]
fn late_unsupported_command_missing_state_and_budgets_refuse_before_resource_reads() {
    let memory = source();
    let mut bytes = commands();
    bytes.extend(0xc000_2d00_u32.to_le_bytes());
    assert!(matches!(
        prepare(&bytes, &memory, &Limits::default()),
        Err(Error::Packet { .. })
    ));
    assert_eq!(memory.reads.get(), 0);
    assert!(matches!(
        prepare(
            &commands(),
            &memory,
            &Limits {
                max_host_bytes: 0,
                ..Default::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    assert_eq!(memory.reads.get(), 0);
    let only_dispatch: Vec<u8> = [0xc003_1502_u32, 1, 1, 1, 0x8005]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert!(matches!(
        prepare(&only_dispatch, &memory, &Limits::default()),
        Err(Error::MissingRegister(_))
    ));
    assert_eq!(memory.reads.get(), 0);
    assert!(matches!(
        prepare(
            &commands(),
            &memory,
            &Limits {
                compiler_phase_bytes: 0,
                ..Default::default()
            }
        ),
        Err(Error::Limit(_))
    ));
    assert_eq!(memory.reads.get(), 0);
}
#[test]
fn shader_termination_checks_instruction_boundaries_and_refuses_back_edges() {
    let mut memory = source();
    memory.words.splice(..0, [0xbe86_03ff, 0xbf81_0000]);
    assert_eq!(
        prepare(&commands(), &memory, &Limits::default())
            .unwrap()
            .usage()
            .shader_words,
        7
    );
    memory.words = vec![0xbf82_ffff, 0xbf81_0000];
    assert!(matches!(
        prepare(&commands(), &memory, &Limits::default()),
        Err(Error::Unsupported("unbounded compute shader loop"))
    ));
}
