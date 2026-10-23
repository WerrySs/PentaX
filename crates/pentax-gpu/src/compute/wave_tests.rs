// SPDX-License-Identifier: GPL-2.0-only
//! Original register/shader consumers for the exact compute wave-width modes.
use super::*;
use pentax_shader_compiler::InputInterface;
use std::cell::Cell;

const CODE: u64 = 0x10000;
const DATA: u64 = 0x20000;
struct Source {
    code: Vec<u32>,
    reads: Cell<usize>,
    data_reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.set(self.reads.get() + 1);
        if address == DATA {
            self.data_reads.set(self.data_reads.get() + 1);
            output.fill(0xa5);
            return Ok(());
        }
        let offset = address.checked_sub(CODE).ok_or(MemoryError::Unavailable)?;
        if output.len() != 4 || !offset.is_multiple_of(4) {
            return Err(MemoryError::Unavailable);
        }
        let word = self
            .code
            .get(offset as usize / 4)
            .ok_or(MemoryError::Unavailable)?;
        output.copy_from_slice(&word.to_le_bytes());
        Ok(())
    }
}
fn source(dynamic_exec: bool) -> Source {
    // Define both vector destinations while every lane is active. Masked
    // vector writes preserve their old destination on inactive lanes.
    let mut code = vec![
        (0x25 << 25) | (1 << 17) | 137,
        (0x1a << 25) | (2 << 17) | 130,
    ];
    if dynamic_exec {
        // S_MOV_B64 EXEC, s[4:5]: both halves are real user-data inputs.
        code.push(0xbefe_0404);
    }
    code.extend([0xe070_1000, (128 << 24) | (1 << 8) | 2, 0xbf81_0000]);
    Source {
        code,
        reads: Cell::new(0),
        data_reads: Cell::new(0),
    }
}
fn state() -> ComputeState {
    let mut state = ComputeState::empty();
    for (register, value) in [
        (0x204, 0),
        (0x205, 0),
        (0x206, 0),
        (0x207, 64),
        (0x208, 1),
        (0x209, 1),
        (0x20c, (CODE >> 8) as u32),
        (0x20d, 0),
        (0x212, 0x2c_0000),
        (0x213, 12),
        (0x228, 0),
        (0x240, DATA as u32),
        (0x241, 0),
        (0x242, 256),
        (0x243, 0),
        (0x244, 0xaaaa_5555),
        (0x245, 0x5555_aaaa),
    ] {
        state.write(register, value).unwrap();
    }
    state
}

#[test]
fn both_wave_widths_compile_all_active_guest_shaders_without_inventing_subgroup_needs() {
    for mode in [1, 5, 0x41, 0x8001, 0x8005, 0x8041] {
        let memory = source(false);
        let dispatch = Dispatch {
            state: state(),
            groups: [1, 1, 1],
            mode,
            indirect: None,
        };
        let (prepared, usage) = prepare_dispatch(&dispatch, &memory, &Limits::default())
            .unwrap()
            .into_parts();
        let prepared = prepared.unwrap();
        assert_eq!(prepared.groups(), [1, 1, 1]);
        assert_eq!(usage.invocations, 64);
        assert_eq!(prepared.input().unwrap(), &[0xa5; 256]);
        assert_eq!(
            prepared.shader().manifest().workgroup_size(),
            Some([64, 1, 1])
        );
        assert_eq!(prepared.shader().manifest().required_subgroup_size(), None);
        assert!(
            !prepared
                .shader()
                .manifest()
                .required_features()
                .subgroup_basic()
        );
        assert!(
            !prepared
                .shader()
                .manifest()
                .required_features()
                .subgroup_ballot()
        );
        assert!(memory.reads.get() > 0);
    }
}

#[test]
fn live_exec_halves_use_portable_compute64_lanes_and_keep_native_wave32() {
    let mut modules = Vec::new();
    for (mode, lanes) in [(0x41, 64), (0x8041, 32)] {
        let memory = source(true);
        let dispatch = Dispatch {
            state: state(),
            groups: [1, 1, 1],
            mode,
            indirect: None,
        };
        let (prepared, _) = prepare_dispatch(&dispatch, &memory, &Limits::default())
            .unwrap()
            .into_parts();
        let prepared = prepared.unwrap();
        let manifest = prepared.shader().manifest();
        if lanes == 64 {
            assert_eq!(manifest.required_subgroup_size(), None);
            assert!(!manifest.required_features().subgroup_basic());
            assert!(!manifest.required_features().subgroup_ballot());
            assert_eq!(
                manifest
                    .inputs()
                    .iter()
                    .filter(|input| **input == InputInterface::LocalInvocationIndex)
                    .count(),
                1
            );
            assert!(
                !manifest
                    .inputs()
                    .contains(&InputInterface::SubgroupLocalInvocationId)
            );
        } else {
            assert_eq!(manifest.required_subgroup_size(), Some(32));
            assert!(manifest.required_features().subgroup_basic());
            assert!(
                manifest
                    .inputs()
                    .contains(&InputInterface::SubgroupLocalInvocationId)
            );
        }
        assert_eq!(manifest.undefined_register_reads(), 0);
        modules.push(prepared.shader().spirv().to_vec());
    }
    assert_ne!(modules[0], modules[1]);
}

#[test]
fn wave64_keeps_register_guards_before_fetch_and_descriptors_before_payload_reads() {
    let original = state();
    for (register, value, reason) in [
        (0x228, 1, "compute resource configuration"),
        (0x213, 12 | 1, "compute resource configuration"),
        (0x243, 1, "raw unswizzled buffer descriptor"),
    ] {
        let mut state = original;
        state.write(register, value).unwrap();
        let memory = source(false);
        let dispatch = Dispatch {
            state,
            groups: [1, 1, 1],
            mode: 0x41,
            indirect: None,
        };
        assert!(
            matches!(prepare_dispatch(&dispatch, &memory, &Limits::default()),
            Err(Error::Unsupported(actual)) if actual == reason)
        );
        if register == 0x243 {
            assert!(memory.reads.get() > 0); // Shader discovery legitimately comes first.
            assert_eq!(memory.data_reads.get(), 0);
        } else {
            assert_eq!(memory.reads.get(), 0);
        }
    }
    let memory = source(false);
    let dispatch = Dispatch {
        state: original,
        groups: [1, 1, 1],
        mode: 0x21,
        indirect: None,
    };
    assert!(matches!(
        prepare_dispatch(&dispatch, &memory, &Limits::default()),
        Err(Error::Unsupported("compute dispatch initiator"))
    ));
    let dispatch = Dispatch {
        mode: 0x41,
        ..dispatch
    };
    let limits = Limits {
        max_invocations: 63,
        ..Default::default()
    };
    assert!(matches!(
        prepare_dispatch(&dispatch, &memory, &limits),
        Err(Error::Limit("dispatch invocations"))
    ));
    assert_eq!(memory.reads.get(), 0);
    assert_eq!(original, state());
}
