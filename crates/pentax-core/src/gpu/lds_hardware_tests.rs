// SPDX-License-Identifier: GPL-2.0-only
//! Original guest LDS shader: real Workgroup sharing across logical Wave64
//! halves and the second Y row. No host shader or hardware-skip oracle.
use super::*;
use pentax_gpu::graphics::InputInterface;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

const PAGE: usize = 65_536;
const TARGET_OFFSET: usize = 256;
const INVOCATIONS: usize = 128;
const OUTPUT_BYTES: usize = INVOCATIONS * 4;
const CANARY: u8 = 0xa5;
const INITIAL: u32 = 0xf00d_1234;
const MASK: [u32; 2] = [0x8000_0000, 0x8000_0001];

fn words_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn binary(op: u32, destination: u32, scalar: u32, vector: u32) -> u32 {
    (op << 25) | (destination << 17) | (vector << 9) | scalar
}
fn shader() -> Vec<u32> {
    vec![
        binary(0x1a, 2, 134, 1), // v2 = localY << 6.
        binary(0x25, 3, 258, 0), // v3 = localX + v2: full x-major index.
        binary(0x25, 4, 137, 3), // v4 = index + 9, every written LDS word differs.
        binary(0x1a, 5, 130, 3), // v5 = index << 2, LDS/output byte address.
        binary(0x1d, 6, 160, 3), // v6 = index xor32, other logical-wave half.
        binary(0x1a, 6, 130, 6), // v6 = neighbor byte address, preserves Y row.
        0x7e0e_02ff,
        INITIAL, // V_MOV_B32 v7, literal: all lanes initialized.
        0xd834_0000,
        (4 << 8) | 5, // DS_WRITE_B32 v5,v4, full EXEC, all128 words.
        0xbf8a_0000,  // Genuine S_BARRIER across the whole workgroup.
        0xbefe_0404,  // EXEC s[4:5]: lanes31/32/63 in each logical wave.
        0xd8d8_0000,
        (7 << 24) | 6, // DS_READ_B32 v7,v6, actual shared neighbor.
        0xbefe_04c1,   // Restore EXEC=-1 so every output word is written.
        0xe070_1000,
        (128 << 24) | (7 << 8) | 5, // BUFFER_STORE_DWORD v7 atv5.
        0xbf81_0000,
    ]
}
fn commands(code: u64, data: u64) -> Vec<u32> {
    let mut words = Vec::new();
    let mut set = |first: u32, values: &[u32]| {
        words.push(0xc000_7600 | ((values.len() as u32) << 16));
        words.push(first);
        words.extend_from_slice(values);
    };
    set(0x204, &[0, 0, 0]);
    set(0x207, &[64, 2, 1]);
    set(0x20c, &[(code >> 8) as u32, (code >> 40) as u32]);
    // Eight VGPRs (v0..v7), six user SGPRs, localX/localY,512 actual LDS bytes.
    set(0x212, &[0x2c_0001, 12 | (1 << 11) | (1 << 15)]);
    set(0x228, &[0]);
    set(
        0x240,
        &[
            data as u32,
            (data >> 32) as u32,
            OUTPUT_BYTES as u32,
            0,
            MASK[0],
            MASK[1],
        ],
    );
    words.extend([0xc003_1500, 1, 1, 1, 0x41]); // Guest Wave64, not native64.
    words
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    memory.read(address, &mut bytes).unwrap();
    bytes
}

#[test]
#[ignore = "requires explicit real Vulkan hardware execution and checked compute LDS admission"]
fn actual_logical_wave64_lds_cross_half_and_y_row_reads_reach_guarded_ram() {
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
    let mut command_page = vec![CANARY; PAGE];
    let mut code_page = vec![CANARY; PAGE];
    let code_bytes = words_bytes(&shader());
    code_page[256..256 + code_bytes.len()].copy_from_slice(&code_bytes);
    let command_bytes = words_bytes(&commands(code, data));
    command_page[4096..4096 + command_bytes.len()].copy_from_slice(&command_bytes);
    let root = compute::CommandRange {
        address: base + 4096,
        size: command_bytes.len() as u64,
    };
    {
        let mut memory = owner.pause().unwrap();
        memory.write(base, &command_page).unwrap();
        memory.write(shader_page, &code_page).unwrap();
        memory.write(data_page, &vec![CANARY; PAGE]).unwrap();
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
    let plan = compute::plan_commands(
        &compute::ComputeState::empty(),
        root,
        &Memory(&memory),
        &compute::CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.dispatches().len(), 1);
    let dispatch = &plan.dispatches()[0];
    let (prepared, _) = compute::prepare_dispatch(dispatch, &Memory(&memory), &limits)
        .expect("original LDS shader must compile before physical submission")
        .into_parts();
    let prepared = prepared.expect("nonempty original LDS dispatch");
    assert_eq!(prepared.buffers().len(), 1);
    let manifest = prepared.shader().manifest();
    assert_eq!(manifest.workgroup_size(), Some([64, 2, 1]));
    assert_eq!(manifest.shared_memory_bytes(), 512);
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
    .expect("hardware consumer requires a real successful fence and owned writeback");
    assert!(admitted);
    assert_eq!(report.preparation.invocations, INVOCATIONS as u64);
    assert_eq!(report.buffer_bytes, OUTPUT_BYTES);
    assert_eq!(report.written_bytes, OUTPUT_BYTES);
    assert!(report.preflight_work > 0);
    assert!(report.gpu.commands > 0 && report.gpu.shader_words > 0 && report.gpu.device_bytes > 0);
    let mut expected = vec![CANARY; PAGE];
    for index in 0..INVOCATIONS {
        let lane = index % 64;
        let active = MASK[lane / 32] & (1u32 << (lane % 32)) != 0;
        let word = if active {
            (index as u32 ^ 32) + 9
        } else {
            INITIAL
        };
        let at = TARGET_OFFSET + index * 4;
        expected[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    // Every output word is checked: inactive lanes retain the shader literal,
    // not the initial RAM canary. The selected six results are72/9/40 and
    //136/73/104, distinguishing lost Y, repeated low EXEC and fake LDS zero.
    assert_eq!(read(&memory, data_page, PAGE), expected);
    assert_eq!(read(&memory, base, PAGE), command_page);
    assert_eq!(read(&memory, shader_page, PAGE), code_page);
    println!(
        "lds_gpu_available=true lds_gpu_dispatches=1 lds_gpu_invocations=128 lds_gpu_shared_bytes=512 lds_gpu_cross_half_checked=true lds_gpu_y_row_checked=true lds_gpu_native64_required=false"
    );
}
