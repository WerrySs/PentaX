// SPDX-License-Identifier: GPL-2.0-only
//! Original physical GPU consumer for compute logical-wave64 mask selection.
//! Requires the separately reviewed portable LaneId lowering, never a skip.
use super::*;
use pentax_gpu::graphics::InputInterface;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

const PAGE: usize = 65536;
const TARGET_OFFSET: usize = 256;
const LANES: usize = 128;
const OUTPUT_BYTES: usize = LANES * 4;
const CANARY: u8 = 0xa5;

fn words_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn binary(op: u32, destination: u32, scalar: u32, vector: u32) -> u32 {
    (op << 25) | (destination << 17) | (vector << 9) | scalar
}
fn shader() -> Vec<u32> {
    vec![
        binary(0x1a, 2, 134, 1), // v2 = localY << 6.
        binary(0x25, 3, 258, 0), // v3 = v2 + localX: full x-major index.
        binary(0x25, 4, 137, 3), // v4 = index + 9: distinguish all outputs.
        binary(0x1a, 5, 130, 3), // v5 = index << 2: byte offset.
        0xbefe_0404,             // S_MOV_B64 EXEC, s[4:5], both dynamic halves.
        0xe070_1000,             // BUFFER_STORE_DWORD with actual v5/v4.
        (128 << 24) | (4 << 8) | 5,
        0xbf81_0000,
    ]
}
fn commands(code: u64, data: u64, mask: [u32; 2]) -> Vec<u32> {
    let mut words = Vec::new();
    let mut set = |first: u32, values: &[u32]| {
        words.push(0xc000_7600 | ((values.len() as u32) << 16));
        words.push(first);
        words.extend_from_slice(values);
    };
    set(0x204, &[0, 0, 0]);
    set(0x207, &[64, 2, 1]); // Two logical waves, including a y transition.
    set(0x20c, &[(code >> 8) as u32, (code >> 40) as u32]);
    set(0x212, &[0x2c_0001, 12 | (1 << 11)]); // Eight allocated VGPRs, six user SGPRs, localX/localY.
    set(0x228, &[0]);
    set(
        0x240,
        &[
            data as u32,
            (data >> 32) as u32,
            OUTPUT_BYTES as u32,
            0,
            mask[0],
            mask[1],
        ],
    );
    words.extend([0xc003_1500, 1, 1, 1, 0x41]); // Genuine guest wave64.
    words
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    memory.read(address, &mut bytes).unwrap();
    bytes
}

#[test]
#[ignore = "requires explicit real Vulkan hardware execution and portable compute-wave64 compiler"]
fn actual_portable_wave64_exec_halves_write_real_guarded_ram_across_two_logical_waves() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let shader_page = base + PAGE as u64;
    let data_page = base + 2 * PAGE as u64;
    let code = shader_page + 256;
    let data = data_page + TARGET_OFFSET as u64;
    for (address, writable) in [(base, false), (shader_page, false), (data_page, true)] {
        owner
            .map_at(
                address,
                PAGE as u64,
                NativePermissions::READ_WRITE.with_gpu(true, writable),
            )
            .unwrap();
    }
    // Asymmetric halves catch native32 repetition, ignored high bits, wrong
    // word selection and failure to restart the logical lane after index63.
    let masks = [[0x8000_0000, 0x8000_0001], [0x4000_0001, 0x0000_0002]];
    let mut command_page = vec![CANARY; PAGE];
    let mut code_page = vec![CANARY; PAGE];
    let code_bytes = words_bytes(&shader());
    code_page[256..256 + code_bytes.len()].copy_from_slice(&code_bytes);
    let mut roots = Vec::new();
    for (index, mask) in masks.iter().enumerate() {
        let bytes = words_bytes(&commands(code, data, *mask));
        let offset = 4096 * (index + 1);
        command_page[offset..offset + bytes.len()].copy_from_slice(&bytes);
        roots.push(compute::CommandRange {
            address: base + offset as u64,
            size: bytes.len() as u64,
        });
    }
    {
        let mut memory = owner.pause().unwrap();
        memory.write(base, &command_page).unwrap();
        memory.write(shader_page, &code_page).unwrap();
    }
    for address in [base, shader_page] {
        owner
            .protect(
                address,
                PAGE as u64,
                NativePermissions::READ.with_gpu(true, false),
            )
            .unwrap();
    }
    let mut memory = owner.pause().unwrap();
    let limits = Limits::default();
    let gpu_limits = GpuLimits::default();
    for (root, mask) in roots.iter().zip(masks) {
        memory.write(data_page, &vec![CANARY; PAGE]).unwrap();
        let plan = compute::plan_commands(
            &compute::ComputeState::empty(),
            *root,
            &Memory(&memory),
            &compute::CommandLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.dispatches().len(), 1);
        let dispatch = &plan.dispatches()[0];
        let (prepared, _) = compute::prepare_dispatch(dispatch, &Memory(&memory), &limits)
            .expect("original wave64 shader must compile before physical submission")
            .into_parts();
        let prepared = prepared.expect("nonempty original compute dispatch");
        let manifest = prepared.shader().manifest();
        assert_eq!(manifest.workgroup_size(), Some([64, 2, 1]));
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
        drop(prepared);
        let mut admitted = false;
        let report = execute_dispatch_admitted(
            &mut memory,
            dispatch,
            &limits,
            &gpu_limits,
            plan.command_ranges(),
            |_, _, may_execute| {
                assert!(may_execute);
                assert!(!admitted);
                admitted = true;
                Ok(())
            },
        )
        .expect("explicit hardware test requires a real successful GPU fence and owned writeback");
        assert!(admitted);
        assert_eq!(report.preparation.invocations, LANES as u64);
        assert_eq!(report.buffer_bytes, OUTPUT_BYTES);
        assert_eq!(report.written_bytes, OUTPUT_BYTES);
        assert!(report.preflight_work > 0);
        assert!(
            report.gpu.commands > 0 && report.gpu.shader_words > 0 && report.gpu.device_bytes > 0
        );
        let mut expected = vec![CANARY; PAGE];
        for index in 0..LANES {
            let lane = index % 64;
            if mask[lane / 32] & (1u32 << (lane % 32)) != 0 {
                let at = TARGET_OFFSET + index * 4;
                expected[at..at + 4].copy_from_slice(&(index as u32 + 9).to_le_bytes());
            }
        }
        assert_eq!(read(&memory, data_page, PAGE), expected);
        assert_eq!(read(&memory, base, PAGE), command_page);
        assert_eq!(read(&memory, shader_page, PAGE), code_page);
    }
    // Emit only after both real fences, exact high-half results and all guards.
    println!(
        "portable_wave64_gpu_available=true portable_wave64_gpu_dispatches=2 portable_wave64_gpu_logical_lanes=64 portable_wave64_gpu_native64_required=false portable_wave64_gpu_high_half_checked=true"
    );
}
