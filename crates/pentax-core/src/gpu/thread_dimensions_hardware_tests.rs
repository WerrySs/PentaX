// SPDX-License-Identifier: GPL-2.0-only
//! Original real GPU thread-dimension consumer. No guest/private shader data.
//! Root must bind the separately attested native32 device and actual compiler/
//! adapter dependencies before execution; this source is not hardware proof.
use super::*;
use pentax_gpu::graphics::InputInterface;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

const PAGE: usize = 65536;
const PADDED_THREADS: usize = 32 * 16;
const ONE_OUTPUT_BYTES: usize = PADDED_THREADS * 4;
const BUFFER_BYTES: usize = 4 * ONE_OUTPUT_BYTES;
const DATA_OFFSET: usize = 256;
const CANARY: u8 = 0xa5;
// Public GFX10.3 named field positions; synthetic values, not observed game R1.
// The legacy allocation nibble is compatibility metadata, not USER/SGPR authority.
const LEGACY_SGPRS_SHIFT: u32 = 6;
const MEM_ORDERED: u32 = 1 << 30;
const THREAD_DIMENSIONS: u32 = 1 << 5;
// Independently derived literal EXEC words, ordered (groupY,groupX,logicalWave).
// Local16x8 makes two logical64 waves per workgroup. Requested X19 leaves
// three x lanes per16-wide row; X17 leaves one. Y10 leaves two final rows,
// Y9 one. These words are consumed by real guest scalar EXEC reads.
const INITIAL_MASK_WORDS: [[[u32; 2]; 8]; 2] = [
    [
        [u32::MAX, u32::MAX],
        [u32::MAX, u32::MAX],
        [0x0007_0007, 0x0007_0007],
        [0x0007_0007, 0x0007_0007],
        [u32::MAX, 0],
        [0, 0],
        [0x0007_0007, 0],
        [0, 0],
    ],
    [
        [u32::MAX, u32::MAX],
        [u32::MAX, u32::MAX],
        [0x0001_0001, 0x0001_0001],
        [0x0001_0001, 0x0001_0001],
        [0x0000_ffff, 0],
        [0, 0],
        [1, 0],
        [0, 0],
    ],
];

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn unary(op: u32, destination: u32, source: u32) -> u32 {
    0x7e00_0000 | (destination << 17) | (op << 9) | source
}
fn binary(op: u32, destination: u32, source: u32, vector: u32) -> u32 {
    (op << 25) | (destination << 17) | (vector << 9) | source
}
fn coordinates(words: &mut Vec<u32>) {
    // Local IDs are actual v0/v1. The four USER words are the buffer descriptor;
    // enabled workgroup X/Y IDs follow at s4/s5. Compute global y*32+x, not a
    // native subgroup lane. Every variable used after reactivation is freshly
    // recomputed from these initialized entry values; inactive old data is unused.
    words.extend([
        unary(1, 2, 5),          // v2=groupY.
        binary(0x1a, 2, 131, 2), // v2=groupY*8.
        binary(0x25, 2, 257, 2), // v2=globalY.
        unary(1, 3, 4),          // v3=groupX.
        binary(0x1a, 3, 132, 3), // v3=groupX*16.
        binary(0x25, 3, 256, 3), // v3=globalX.
        binary(0x1a, 4, 133, 2), // v4=globalY*32.
        binary(0x25, 4, 259, 4), // v4=globalY*32+globalX (0..511).
        binary(0x25, 5, 137, 4), // v5=index+9, distinguishing all words.
        binary(0x1a, 6, 130, 4), // v6=index*4, the actual byte offset.
    ]);
}
fn shader() -> Vec<u32> {
    let mut words = vec![0xbe86_037e, 0xbe87_037f]; // S_MOV_B32 s6=EXEC_LO, s7=EXEC_HI.
    // Deliberately normal first VGPR definitions under the actual initial EXEC.
    // This needs the compiler's defined-on-active initial-tail qualification;
    // no full-EXEC initialization workaround is used to conceal that boundary.
    coordinates(&mut words);
    words.extend([
        0xe070_1000,
        (128 << 24) | (5 << 8) | 6, // First output: only initial valid threads.
        0xbf8a_0000, // Unconditional real S_BARRIER, including inactive/padded lanes.
        0xbefe_04c1, // Guest S_MOV_B64 EXEC,-1 genuinely reactivates both halves.
    ]);
    coordinates(&mut words); // Full overwrite; no previous inactive VGPR is read.
    words.extend([
        0xe070_1000 | ONE_OUTPUT_BYTES as u32,
        (128 << 24) | (5 << 8) | 6, // Second output: all512 padded invocations.
        unary(1, 7, 6),             // Actual saved EXEC_LO -> v7, all lanes defined now.
        unary(1, 8, 7),             // Actual saved EXEC_HI -> v8.
        binary(0x25, 6, 255, 6),
        4096, // Third region address = index*4+4096.
        0xe070_1000,
        (128 << 24) | (7 << 8) | 6,
        binary(0x25, 6, 255, 6),
        2048, // Fourth region address = index*4+6144.
        0xe070_1000,
        (128 << 24) | (8 << 8) | 6,
        0xbf81_0000,
    ]);
    words
}
fn commands(code: u64, data: u64, requested: [u32; 3]) -> Vec<u32> {
    let mut words = Vec::new();
    let mut set = |first: u32, values: &[u32]| {
        words.push(0xc000_7600 | ((values.len() as u32) << 16));
        words.push(first);
        words.extend_from_slice(values);
    };
    set(0x204, &[0, 0, 0]);
    set(0x207, &[16, 8, 1]);
    set(0x20c, &[(code >> 8) as u32, (code >> 40) as u32]);
    // Allocation encoding2, v0..v8 used; fixed numeric profile remains strict.
    // Legacy SGPRS5 does not enlarge the four USER words. MEM_ORDERED is the
    // admitted non-sampled memory domain, not a substitute for a guest barrier.
    set(
        0x212,
        &[
            0x002c_0002 | (5 << LEGACY_SGPRS_SHIFT) | MEM_ORDERED,
            8 | 0x80 | 0x100 | (1 << 11),
        ],
    ); // USER4, groupX/Y, localX/Y.
    set(0x228, &[0]);
    set(
        0x240,
        &[data as u32, (data >> 32) as u32, BUFFER_BYTES as u32, 0],
    );
    words.extend([
        0xc003_1500,
        requested[0],
        requested[1],
        requested[2],
        0x41 | THREAD_DIMENSIONS,
    ]);
    words
}
fn read(memory: &PausedNativeMemory<'_>, at: u64, length: usize) -> Vec<u8> {
    let mut out = vec![0; length];
    memory.read(at, &mut out).unwrap();
    out
}

#[test]
#[ignore = "requires real Vulkan native32 proof and qualified compute thread-dimensions compiler/adapter"]
fn actual_thread_dimensions_wave64_masks_both_halves_and_reactivates_padded_threads() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let code_page = base + PAGE as u64;
    let data_page = base + 2 * PAGE as u64;
    let code = code_page + 256;
    let data = data_page + DATA_OFFSET as u64;
    for (at, writable) in [(base, false), (code_page, false), (data_page, true)] {
        owner
            .map_at(
                at,
                PAGE as u64,
                NativePermissions::READ_WRITE.with_gpu(true, writable),
            )
            .unwrap();
    }
    let requests = [[19, 10, 1], [17, 9, 1]];
    let mut command_bytes = vec![CANARY; PAGE];
    let mut code_bytes = vec![CANARY; PAGE];
    let program = bytes(&shader());
    code_bytes[256..256 + program.len()].copy_from_slice(&program);
    let mut roots = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        let encoded = bytes(&commands(code, data, *request));
        let offset = 4096 * (index + 1);
        command_bytes[offset..offset + encoded.len()].copy_from_slice(&encoded);
        roots.push(compute::CommandRange {
            address: base + offset as u64,
            size: encoded.len() as u64,
        });
    }
    {
        let mut memory = owner.pause().unwrap();
        memory.write(base, &command_bytes).unwrap();
        memory.write(code_page, &code_bytes).unwrap();
    }
    for at in [base, code_page] {
        owner
            .protect(
                at,
                PAGE as u64,
                NativePermissions::READ.with_gpu(true, false),
            )
            .unwrap();
    }
    let mut memory = owner.pause().unwrap();
    let limits = Limits::default();
    let gpu_limits = GpuLimits::default();
    let mut previous_shader: Option<Vec<u32>> = None;
    let mut previous_push: Option<[u8; 128]> = None;
    let mut previous_slot: Option<usize> = None;
    for (request_index, (root, request)) in roots.iter().zip(requests).enumerate() {
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
            .unwrap()
            .into_parts();
        let prepared = prepared.expect("positive requested thread dimensions require real work");
        assert_eq!(prepared.groups(), [2, 2, 1]);
        assert_eq!(prepared.thread_extent(), Some(request));
        assert_eq!(prepared.usage().invocations, PADDED_THREADS as u64);
        assert_eq!(prepared.resource_controls().legacy_sgprs(), 5);
        assert!(prepared.resource_controls().mem_ordered());
        let manifest = prepared.shader().manifest();
        assert_eq!(manifest.workgroup_size(), Some([16, 8, 1]));
        assert_eq!(manifest.required_subgroup_size(), None);
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
        assert!(manifest.images().is_empty() && manifest.samplers().is_empty());
        let relative = manifest
            .dispatch_thread_extent_dword()
            .expect("live initial EXEC needs all three extents");
        let layout = manifest
            .push_data()
            .expect("declared runtime extent layout");
        let slot = ((layout.start_dword + relative) * 4) as usize;
        let push = *prepared.push_data();
        let extent_bytes = bytes(&request);
        assert_eq!(&push[slot..slot + 12], extent_bytes.as_slice());
        if let Some(old) = &previous_shader {
            assert_eq!(prepared.shader().spirv(), old.as_slice()); // Extents are runtime, not baked shader constants.
            assert_eq!(previous_slot, Some(slot));
            let mut changed = previous_push.unwrap();
            changed[slot..slot + 12].copy_from_slice(&bytes(&request));
            assert_eq!(push, changed); // Exactly the three declared extent words can change.
        }
        previous_shader = Some(prepared.shader().spirv().to_vec());
        previous_push = Some(push);
        previous_slot = Some(slot);
        drop(prepared);
        let mut admitted = false;
        let report = execute_dispatch_admitted(
            &mut memory,
            dispatch,
            &limits,
            &gpu_limits,
            plan.command_ranges(),
            |prepared, _, may_execute| {
                assert_eq!(prepared.invocations, PADDED_THREADS as u64);
                assert!(may_execute);
                assert!(!admitted);
                admitted = true;
                Ok(())
            },
        )
        .expect("requires a real completed GPU fence and owned buffer writeback");
        assert!(admitted);
        assert_eq!(report.preparation.invocations, PADDED_THREADS as u64);
        assert_eq!(report.buffer_bytes, BUFFER_BYTES);
        assert_eq!(report.written_bytes, BUFFER_BYTES);
        assert!(
            report.gpu.commands > 0 && report.gpu.shader_words > 0 && report.gpu.device_bytes > 0
        );
        let mut expected = vec![CANARY; PAGE];
        let mut active = 0;
        for index in 0..PADDED_THREADS {
            let x = (index % 32) as u32;
            let y = (index / 32) as u32;
            let value = (index as u32 + 9).to_le_bytes();
            if x < request[0] && y < request[1] {
                active += 1;
                expected[DATA_OFFSET + index * 4..DATA_OFFSET + index * 4 + 4]
                    .copy_from_slice(&value);
            }
            // EXEC=-1 and fresh definitions must include padded lanes too.
            let at = DATA_OFFSET + ONE_OUTPUT_BYTES + index * 4;
            expected[at..at + 4].copy_from_slice(&value);
            let mask_index = (((y / 8) * 2 + x / 16) * 2 + (y % 8) / 4) as usize;
            let mask = INITIAL_MASK_WORDS[request_index][mask_index];
            for (half, mask_word) in mask.into_iter().enumerate() {
                let at = DATA_OFFSET + (2 + half) * ONE_OUTPUT_BYTES + index * 4;
                expected[at..at + 4].copy_from_slice(&mask_word.to_le_bytes());
            }
        }
        assert_eq!(active, (request[0] * request[1]) as usize);
        assert_eq!(active, if request[0] == 19 { 190 } else { 153 });
        assert_eq!(read(&memory, data_page, PAGE), expected);
        assert_eq!(read(&memory, base, PAGE), command_bytes);
        assert_eq!(read(&memory, code_page, PAGE), code_bytes);
    }
    println!(
        "thread_dimensions_gpu_available=true thread_dimensions_gpu_dispatches=2 thread_dimensions_gpu_wave64=true thread_dimensions_gpu_padded_invocations=1024 thread_dimensions_gpu_initial_active=343 thread_dimensions_gpu_initial_exec_words=true thread_dimensions_gpu_exec_reactivation=true thread_dimensions_gpu_runtime_extents=true thread_dimensions_gpu_guards=true"
    );
}
