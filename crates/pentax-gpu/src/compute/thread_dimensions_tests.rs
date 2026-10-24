// SPDX-License-Identifier: GPL-2.0-only
//! Original geometry/profile/runtime-push consumers; no GPU or guest bytes.
use super::*;
use std::cell::{Cell, RefCell};
const CODE: u64 = 0x10000;
const DATA: u64 = 0x20000;
const ARGS: u64 = 0x30000;
struct Source {
    code: Vec<u32>,
    args: RefCell<[u32; 3]>,
    code_reads: Cell<usize>,
    data_reads: Cell<usize>,
    args_reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        if address == ARGS && output.len() == 12 {
            self.args_reads.set(self.args_reads.get() + 1);
            for (word, value) in output.chunks_exact_mut(4).zip(*self.args.borrow()) {
                word.copy_from_slice(&value.to_le_bytes());
            }
            return Ok(());
        }
        if address == DATA {
            self.data_reads.set(self.data_reads.get() + 1);
            output.fill(0xa5);
            return Ok(());
        }
        let at = address
            .checked_sub(CODE)
            .filter(|at| at.is_multiple_of(4))
            .ok_or(MemoryError::Unavailable)?;
        if output.len() != 4 {
            return Err(MemoryError::Unavailable);
        }
        let word = self
            .code
            .get(at as usize / 4)
            .ok_or(MemoryError::Unavailable)?;
        self.code_reads.set(self.code_reads.get() + 1);
        output.copy_from_slice(&word.to_le_bytes());
        Ok(())
    }
}
fn source() -> Source {
    // v0/v1/v2 are actual XYZ initial inputs, so these masked writes preserve
    // defined old destinations. The live SGPR4 seed must not collide with XYZ.
    Source {
        code: vec![
            (0x25 << 25) | (1 << 17) | 4,
            (0x1a << 25) | (2 << 17) | 130,
            0xe070_1000,
            (128 << 24) | (1 << 8) | 2,
            0xbf81_0000,
        ],
        args: RefCell::new([9, 5, 3]),
        code_reads: Cell::new(0),
        data_reads: Cell::new(0),
        args_reads: Cell::new(0),
    }
}
fn state(local: [u32; 3], r1: u32) -> ComputeState {
    let mut state = ComputeState::for_new_queue();
    for (reg, value) in [
        (0x207, local[0]),
        (0x208, local[1]),
        (0x209, local[2]),
        (0x20c, (CODE >> 8) as u32),
        (0x20d, 0),
        (0x212, r1),
        (0x213, 10 | (2 << 11)),
        (0x228, 0),
        (0x240, DATA as u32),
        (0x241, 0),
        (0x242, 256),
        (0x243, 0),
        (0x244, 17),
    ] {
        state.write(reg, value).unwrap();
    }
    state
}
fn direct(state: ComputeState, extent: [u32; 3], mode: u32) -> Dispatch {
    Dispatch::from_packet(state, &[0xc003_1500, extent[0], extent[1], extent[2], mode]).unwrap()
}
fn prepare_product(dispatch: &Dispatch, memory: &impl Memory) -> PreparedCompute {
    prepare_dispatch(dispatch, memory, &Limits::default())
        .unwrap()
        .into_parts()
        .0
        .unwrap()
}
#[test]
fn checked_thread_geometry_covers_xyz_ceil_exact_padded_budget_and_axis_overflow() {
    for (threads, local, expected, invocations) in [
        ([64, 1, 1], [64, 1, 1], [1, 1, 1], 64),
        ([65, 1, 1], [64, 1, 1], [2, 1, 1], 128),
        ([19, 10, 1], [16, 8, 1], [2, 2, 1], 512),
        ([9, 5, 3], [8, 4, 2], [2, 2, 2], 512),
        ([1, 1, 1], [8, 4, 2], [1, 1, 1], 64),
    ] {
        assert_eq!(
            dispatch_geometry(threads, local, true, invocations),
            Ok((expected, invocations))
        );
        assert!(matches!(
            dispatch_geometry(threads, local, true, invocations - 1),
            Err(Error::Limit(_))
        ));
    }
    assert_eq!(
        dispatch_geometry([2, 3, 4], [8, 4, 2], false, 1536),
        Ok(([2, 3, 4], 1536))
    );
    assert!(matches!(
        dispatch_geometry([1; 3], [0, 1, 1], true, u64::MAX),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        dispatch_geometry([u32::MAX, 1, 1], [2, 1, 1], true, u64::MAX),
        Err(Error::Limit("dispatch thread extent"))
    ));
    assert!(matches!(
        dispatch_geometry([u32::MAX; 3], [1; 3], true, u64::MAX),
        Err(Error::Limit(_))
    ));
    assert_eq!(
        dispatch_geometry([u32::MAX, 1, 1], [1; 3], true, u64::MAX),
        Ok(([u32::MAX, 1, 1], u64::from(u32::MAX)))
    );
}
#[test]
fn zero_threads_skip_shader_and_resources_but_indirect_counts_are_current_owned_reads() {
    let memory = source();
    for extent in [[0, 1, 1], [1, 0, 1], [1, 1, 0]] {
        let dispatch = direct(ComputeState::empty(), extent, 0x61);
        let (product, usage) = prepare_dispatch(&dispatch, &memory, &Limits::default())
            .unwrap()
            .into_parts();
        assert!(product.is_none());
        assert_eq!(usage.invocations, 0);
        *memory.args.borrow_mut() = extent;
        let indirect =
            Dispatch::from_packet(ComputeState::empty(), &[0xc002_1600, ARGS as u32, 0, 0x61])
                .unwrap();
        let (product, usage) = prepare_dispatch(&indirect, &memory, &Limits::default())
            .unwrap()
            .into_parts();
        assert!(product.is_none());
        assert_eq!(usage.work, 12);
    }
    assert_eq!(memory.code_reads.get(), 0);
    assert_eq!(memory.data_reads.get(), 0);
    assert_eq!(memory.args_reads.get(), 3);
    // Unknown initiator controls still refuse before even the argument read.
    let bad =
        Dispatch::from_packet(ComputeState::empty(), &[0xc002_1600, ARGS as u32, 0, 0x69]).unwrap();
    assert!(matches!(
        prepare_dispatch(&bad, &memory, &Limits::default()),
        Err(Error::Unsupported("compute dispatch initiator"))
    ));
    assert_eq!(memory.args_reads.get(), 3);
}
#[test]
fn actual_compiler_runtime_extents_do_not_alias_user_push_or_specialize_request_values() {
    let memory = source();
    let registers = state([8, 4, 2], 0x2c_0000 | (5 << 6) | (1 << 30));
    let a = prepare_product(&direct(registers, [9, 5, 3], 0x61), &memory);
    let b = prepare_product(&direct(registers, [11, 7, 3], 0x61), &memory);
    assert_eq!(a.groups(), [2, 2, 2]);
    assert_eq!(b.groups(), [2, 2, 2]);
    assert_eq!(a.usage().invocations, 512);
    assert_eq!(b.usage().invocations, 512);
    assert_eq!(a.shader().spirv(), b.shader().spirv());
    assert_ne!(a.push_data(), b.push_data());
    for (prepared, extent) in [(&a, [9, 5, 3]), (&b, [11, 7, 3])] {
        assert_eq!(prepared.thread_extent(), Some(extent));
        assert_eq!(prepared.resource_controls().legacy_sgprs(), 5);
        assert!(prepared.resource_controls().mem_ordered());
        assert!(prepared.shader().manifest().images().is_empty());
        assert!(prepared.shader().manifest().samplers().is_empty());
        let manifest = prepared.shader().manifest();
        let push = manifest.push_data().unwrap();
        let relative = manifest
            .dispatch_thread_extent_dword()
            .expect("live EXEC must consume original runtime extents");
        assert_eq!(relative, push.user_sgprs.len() as u32);
        assert_eq!(push.memory_offset_dword, relative + 3);
        for (axis, value) in extent.into_iter().enumerate() {
            let at = (push.start_dword as usize + relative as usize + axis) * 4;
            assert_eq!(&prepared.push_data()[at..at + 4], &value.to_le_bytes());
        }
        let seed = push.user_sgprs.iter().position(|reg| *reg == 4).unwrap();
        let at = (push.start_dword as usize + seed) * 4;
        assert_eq!(&prepared.push_data()[at..at + 4], &17u32.to_le_bytes());
    }
    let indirect = Dispatch::from_packet(registers, &[0xc002_1600, ARGS as u32, 0, 0x61]).unwrap();
    *memory.args.borrow_mut() = [9, 5, 3];
    let c = prepare_product(&indirect, &memory);
    *memory.args.borrow_mut() = [11, 7, 3];
    let d = prepare_product(&indirect, &memory);
    assert_eq!(c.groups(), a.groups());
    assert_eq!(c.push_data(), a.push_data());
    assert_eq!(d.groups(), b.groups());
    assert_eq!(d.push_data(), b.push_data());
    assert_eq!(memory.args_reads.get(), 2);
    let narrow = prepare_product(&direct(registers, [9, 5, 3], 0x8061), &memory);
    assert_eq!(narrow.groups(), a.groups());
    assert_eq!(narrow.thread_extent(), Some([9, 5, 3]));
    assert_eq!(narrow.shader().manifest().workgroup_size(), Some([8, 4, 2]));
    let group_mode = prepare_product(&direct(registers, [2, 2, 2], 0x41), &memory);
    assert_eq!(group_mode.thread_extent(), None);
    assert_eq!(
        group_mode
            .shader()
            .manifest()
            .dispatch_thread_extent_dword(),
        None
    );
    assert_ne!(group_mode.shader().spirv(), a.shader().spirv());
    let strict = prepare_product(
        &direct(state([8, 4, 2], 0x2c_0000), [9, 5, 3], 0x61),
        &memory,
    );
    assert_eq!(strict.shader().spirv(), a.shader().spirv()); // No MEM_ORDERED-derived barrier or SGPR initializer.
}
#[test]
fn named_r1_subset_retains_other_fields_user_undefined_and_sample_refusals() {
    for legacy in [0, 1, 7, 15] {
        for mem in [false, true] {
            let raw = 0x2c_0000 | (legacy << 6) | if mem { 1 << 30 } else { 0 };
            let controls = ResourceControls::decode(raw).unwrap();
            assert_eq!(u32::from(controls.legacy_sgprs()), legacy);
            assert_eq!(controls.mem_ordered(), mem);
        }
    }
    for bit in [10, 11, 12, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 31] {
        assert!(matches!(
            ResourceControls::decode(0x2c_0000 ^ (1 << bit)),
            Err(Error::Unsupported("compute resource configuration"))
        ));
    }
    let memory = source();
    let mut users = state([8, 4, 2], 0x2c_0000 | (15 << 6) | (1 << 30));
    users.write(0x213, 34 | (2 << 11)).unwrap(); // USER17 is independent of legacy allocation.
    assert!(matches!(
        prepare_dispatch(&direct(users, [9, 5, 3], 0x61), &memory, &Limits::default()),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(memory.data_reads.get(), 0);
    assert_eq!(memory.code_reads.get(), 0);
    let mut undefined = source();
    undefined.code[0] = (0x25 << 25) | (1 << 17) | 7; // SGPR7 never initialized.
    assert!(matches!(
        prepare_dispatch(
            &direct(
                state([8, 4, 2], 0x2c_0000 | (15 << 6) | (1 << 30)),
                [9, 5, 3],
                0x61
            ),
            &undefined,
            &Limits::default()
        ),
        Err(Error::Compile)
    ));
    assert_eq!(undefined.data_reads.get(), 0);
    let mut sampled = source();
    sampled.code = vec![0xf090_0f08, 0x0040_0400, 0xbf81_0000];
    assert!(
        prepare_dispatch(
            &direct(state([8, 4, 2], 0x2c_0000 | (1 << 30)), [9, 5, 3], 0x61),
            &sampled,
            &Limits::default()
        )
        .is_err()
    );
    assert_eq!(sampled.data_reads.get(), 0); // MEM_ORDERED never admits sampled-compute backing.
}
