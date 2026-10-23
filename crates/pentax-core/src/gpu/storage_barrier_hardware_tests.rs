// SPDX-License-Identifier: GPL-2.0-only
//! Original guest SSBO store/barrier/load consumer across 128 local invocations.
//! This checks the complete coherent Workgroup barrier sequence, not standalone
//! VSCNT Wave64 behavior or a rendered/presented frame.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

const PAGE: usize = 65_536;
const DATA_OFFSET: usize = 256;
const INVOCATIONS: usize = 128;
const BUFFER_BYTES: usize = 3072;
const INPUT: usize = 128;
const HALF_OUTPUT: usize = 1024;
const Y_OUTPUT: usize = 2048;
const CANARY: u8 = 0xa5;
const HALF_INITIAL: u32 = 0xf00d_1234;
const Y_INITIAL: u32 = 0x0bad_cafe;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn binary(op: u32, dst: u32, scalar: u32, vector: u32) -> u32 {
    (op << 25) | (dst << 17) | (vector << 9) | scalar
}
fn shader() -> Vec<u32> {
    vec![
        binary(0x1a, 2, 134, 1), // v2=localY<<6.
        binary(0x25, 3, 258, 0), // v3=localX+64*localY, full index 0..127.
        binary(0x25, 4, 137, 3), // v4=index+9, all source words differ.
        binary(0x1a, 5, 130, 3), // v5=index<<2, all output byte offsets.
        0x7e0e_02ff,
        HALF_INITIAL, // Initialize v7 before the actual read.
        0x7e10_02ff,
        Y_INITIAL, // Initialize v8, RSRC1 allocation encoding 2 below.
        0xe070_1000 | INPUT as u32,
        (128 << 24) | (4 << 8) | 5, // BUFFER_STORE_DWORD input[index]=v4.
        0xbbfd_0000,                // S_WAITCNT_VSCNT null,0: guest vector-store wait instruction.
        0xbf8a_0000,                // S_BARRIER: all 128 reach coherent Workgroup memory barrier.
        binary(0x1d, 6, 160, 3),    // v6=index xor 32, across the logical half.
        binary(0x1a, 6, 130, 6),
        0xe030_1000 | INPUT as u32,
        (128 << 24) | (7 << 8) | 6, // BUFFER_LOAD_DWORD v7=input[index xor 32].
        binary(0x1d, 6, 192, 3),    // v6=index xor 64, across localY/logical waves.
        binary(0x1a, 6, 130, 6),
        0xe030_1000 | INPUT as u32,
        (128 << 24) | (8 << 8) | 6, // BUFFER_LOAD_DWORD v8=input[index xor 64].
        0xbf8c_0000,                // S_WAITCNT0 before using both loaded destinations.
        0xe070_1000 | HALF_OUTPUT as u32,
        (128 << 24) | (7 << 8) | 5,
        0xe070_1000 | Y_OUTPUT as u32,
        (128 << 24) | (8 << 8) | 5,
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
    // RSRC1 allocation encoding 2 (v0..v8 used), strict prior R1 mode, four user SGPRs,
    // localX/localY. No LDS, scratch, observed flags or guest coherent bits.
    set(0x212, &[0x2c_0002, 8 | (1 << 11)]);
    set(0x228, &[0]);
    set(
        0x240,
        &[data as u32, (data >> 32) as u32, BUFFER_BYTES as u32, 0],
    );
    words.extend([0xc003_1500, 1, 1, 1, 0x41]); // Guest Wave64, not native64.
    words
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, length: usize) -> Vec<u8> {
    let mut out = vec![0; length];
    memory.read(address, &mut out).unwrap();
    out
}

#[test]
#[ignore = "requires explicit Vulkan hardware and coherent compute S_BARRIER storage policy"]
fn actual_storage_barrier_peer_words_cross_logical_halves_and_y_after_real_fence() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let code_page_at = base + PAGE as u64;
    let data_page_at = base + 2 * PAGE as u64;
    let code = code_page_at + 256;
    let data = data_page_at + DATA_OFFSET as u64;
    for (address, writable) in [(base, false), (code_page_at, false), (data_page_at, true)] {
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
    let code_bytes = bytes(&shader());
    code_page[256..256 + code_bytes.len()].copy_from_slice(&code_bytes);
    let command_bytes = bytes(&commands(code, data));
    command_page[4096..4096 + command_bytes.len()].copy_from_slice(&command_bytes);
    let root = compute::CommandRange {
        address: base + 4096,
        size: command_bytes.len() as u64,
    };
    {
        let mut memory = owner.pause().unwrap();
        memory.write(base, &command_page).unwrap();
        memory.write(code_page_at, &code_page).unwrap();
        memory.write(data_page_at, &vec![CANARY; PAGE]).unwrap();
    }
    for address in [base, code_page_at] {
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
    let (product, _) = compute::prepare_dispatch(dispatch, &Memory(&memory), &limits)
        .expect("original SSBO shader must compile before physical submission")
        .into_parts();
    let prepared = product.expect("nonempty original SSBO dispatch");
    assert_eq!(prepared.buffers().len(), 1);
    let manifest = prepared.shader().manifest();
    assert_eq!(manifest.workgroup_size(), Some([64, 2, 1]));
    assert_eq!(manifest.shared_memory_bytes(), 0);
    assert_eq!(manifest.required_subgroup_size(), None);
    // Actual VSCNT emits a host-subgroup barrier: BASIC, without exact native width.
    assert!(manifest.required_features().subgroup_basic());
    assert!(!manifest.required_features().subgroup_ballot());
    assert_eq!(manifest.buffers().len(), 1);
    assert!(manifest.buffers()[0].read && manifest.buffers()[0].written);
    // Effective emitter coherence comes from the surviving workgroup barrier;
    // it neither admits a guest GLC/DLC request nor expands memory permissions.
    assert!(!manifest.buffers()[0].coherent);
    assert!(manifest.buffers()[0].effective_coherent);
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
    .expect("requires real GPU submit/fence/readback and same-owner writeback");
    assert!(admitted);
    assert_eq!(report.preparation.invocations, INVOCATIONS as u64);
    assert_eq!(report.buffer_bytes, BUFFER_BYTES);
    assert_eq!(report.written_bytes, BUFFER_BYTES);
    assert!(report.preflight_work > 0);
    assert!(report.gpu.commands > 0 && report.gpu.shader_words > 0 && report.gpu.device_bytes > 0);
    let mut expected = vec![CANARY; PAGE];
    for index in 0..INVOCATIONS {
        for (region, value) in [
            (INPUT, index as u32 + 9),
            (HALF_OUTPUT, (index as u32 ^ 32) + 9),
            (Y_OUTPUT, (index as u32 ^ 64) + 9),
        ] {
            let at = DATA_OFFSET + region + index * 4;
            expected[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    // All 384 words and all untouched bytes are compared. Half/Y neighbors
    // differ for every index, including 31/32/63/64/95/96/127. Missing stores,
    // default-register or stale-canary reads cannot satisfy this oracle.
    assert_eq!(read(&memory, data_page_at, PAGE), expected);
    assert_eq!(read(&memory, base, PAGE), command_page);
    assert_eq!(read(&memory, code_page_at, PAGE), code_page);
    println!(
        "storage_barrier_gpu_available=true storage_barrier_gpu_dispatches=1 storage_barrier_gpu_invocations=128 storage_barrier_gpu_checked_words=384 storage_barrier_gpu_cross_half_checked=true storage_barrier_gpu_cross_y_checked=true storage_barrier_gpu_effective_coherent=true storage_barrier_gpu_native64_required=false"
    );
}
