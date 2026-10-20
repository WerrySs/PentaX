// SPDX-License-Identifier: GPL-2.0-only

//! Original x64 consumer calls actual scoped imports, then branches on GPU data.
use super::*;

#[test]
fn native_agc_registry_apis_keep_source_int32_error_and_exact_namespaces() {
    for (module, library, version) in [
        ("Graphics5", "Graphics5", 1),
        ("libSceAgc", "libSceAgc", 1),
        ("Graphics5", "Graphics5", 2),
        ("Graphics5Driver", "Graphics5Driver", 1),
        ("AgcDriver", "AgcDriver", 1),
        ("libSceAgcDriver", "libSceAgcDriver", 1),
        ("libSceAgcDriver", "libSceAgcDriver", 2),
    ] {
        let mut p = Program::new();
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(32);
        for api in 0..4 {
            for reg in [7, 6, 2, 1, 8, 9] {
                p.reg(reg, u64::MAX);
            }
            p.call(api);
            p.expect(0x8a6c_9018);
        }
        p.word(32, 0xa5a5_a5a5_a5a5_a5a5);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["X-Nm5KLREeg", "W5z4eZrjEas", "ZLJk9r2+2Aw", "pWLG7WOpVcw"],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        } else if module == "Graphics5Driver" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "unavailable-function");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        }
    }
}

#[test]
fn native_agc_trinity_void_query_preserves_neighbor_bytes_in_exact_scopes() {
    for (module, library, version) in [
        ("Graphics5", "Graphics5", 1),
        ("libSceAgc", "libSceAgc", 1),
        ("Graphics5", "Graphics5", 2),
    ] {
        let mut p = Program::new();
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(32);
        p.ptr(7, 35);
        p.call(0); // Source void: intentionally ignore transport RAX.
        p.word(32, 0xa5a5_a5a5_00a5_a5a5);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(&code, &["BfBDZGbti7A"], module, library, version);
        let (status, fields, text) = probe(&original);
        if version == 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        }
    }
}

#[test]
fn native_register_defaults_return_real_guest_tables_with_source_versions_and_stable_pointers() {
    for (module, library, namespace_version, version, public_count, internal_count) in [
        ("Graphics5", "Graphics5", 1, 0x1_0000_0000_u64, 150, 15),
        ("Graphics5", "Graphics5", 1, u64::MAX, 137, 24),
        ("libSceAgc", "libSceAgc", 1, 13, 137, 24),
        ("Graphics5", "Graphics5", 2, 0, 150, 15),
        ("Graphics5Driver", "Graphics5Driver", 1, 0, 150, 15),
    ] {
        let mut p = Program::new();
        for (api, count) in [(0, public_count), (1, internal_count)] {
            p.reg(7, version);
            p.call(api);
            p.save(0);
            p.code.extend([0x48, 0x8b, 0x40, 56]); // exact count and zero padding
            p.expect(count);
            p.reg(7, version);
            p.call(api);
            p.load(11, 0);
            p.code.extend([0x4c, 0x39, 0xd8]);
            checked(&mut p.code, &mut p.branches);
            p.code.extend([0x48, 0x8b, 0x08]); // descriptor -> first pointer table
            p.code.extend([0x48, 0x8b, 0x09]); // first pointer -> ShaderRegister
            p.code.extend([0x48, 0x8b, 0x01]); // actual source register pair
            p.expect(if api == 0 {
                (13369360_u64 << 32) | 514
            } else if count == 15 {
                (1_u64 << 32) | 437
            } else {
                (2_u64 << 32) | 14
            });
            p.load(0, 0);
            p.code.extend([0x48, 0x8b, 0x48, 48]); // type triplets, not a host pointer
            p.code.extend([0x8b, 0x01]);
            p.expect(if api == 0 {
                3796861037
            } else if count == 15 {
                4119849521
            } else {
                2410999221
            });
        }
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["2JtWUUiYBXs", "wRbq6ZjNop4"],
            module,
            library,
            namespace_version,
        );
        let (status, fields, text) = probe(&original);
        if namespace_version == 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        } else if module == "Graphics5Driver" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "unavailable-function");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        }
    }
}

#[test]
fn native_agc_init_preserves_source_state_and_accepts_versions_in_exact_scopes() {
    for (module, library, version) in [
        ("Graphics5", "Graphics5", 1),
        ("libSceAgc", "libSceAgc", 1),
        ("Graphics5", "Graphics5", 2),
        ("Graphics5Driver", "Graphics5Driver", 1),
    ] {
        let mut p = Program::new();
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(32);
        p.ptr(7, 32);
        p.reg(6, u64::MAX);
        p.call(0);
        p.expect(0);
        p.word(32, 0xa5a5_a5a5_a5a5_a5a5);
        p.reg(7, 0);
        p.reg(6, 0);
        p.call(0);
        p.expect(0);
        p.reg(7, u64::MAX);
        p.reg(6, 0x1_0000_0001);
        p.call(0);
        p.expect(0);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(&code, &["23LRUSvYu1M"], module, library, version);
        let (status, fields, text) = probe(&original);
        if version == 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        } else if module == "Graphics5Driver" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "unavailable-function");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        }
    }
}
const COMMANDS: i32 = 0x4200;
const SHADER: i32 = 0x4400;
const CONTEXT: i32 = 0x4600;
const PACKET: i32 = 0x4610;
const ADDRESSES: i32 = 0x4630;
const SIZES: i32 = 0x4650;
const DATA: i32 = 0x4800;
const NIDS: [&str; 4] = ["b4fpgH5ZXxQ", "Fj7r9EHzF38", "gSRnr79F8tQ", "HF3YllT3mXU"];
fn call(p: &mut Program, slot: i32, _stateful: bool) {
    rip(&mut p.code, 0x800, &[0xff, 0x15], 0x3000 + slot * 8);
    p.calls += 1;
}

fn gpu_ptr(p: &mut Program, reg: u8, target: i32) {
    assert!(target >= 0x4000);
    p.code
        .extend([0x4c | u8::from(reg >= 8), 0x89, 0xe0 | (reg & 7)]); // MOV reg, R12
    p.code
        .extend([0x48 | u8::from(reg >= 8), 0x81, 0xc0 | (reg & 7)]);
    p.code.extend((target - 0x4000).to_le_bytes());
}
fn store(p: &mut Program, at: i32, wide: bool) {
    gpu_ptr(p, 11, at);
    p.code.extend([if wide { 0x49 } else { 0x41 }, 0x89, 0x03]);
}
fn read_gpu_result(p: &mut Program, at: i32, wide: bool) {
    gpu_ptr(p, 11, at);
    p.code.extend([if wide { 0x49 } else { 0x41 }, 0x8b, 0x03]);
}
fn gpu_allocation(p: &mut Program) {
    p.set(96, 0);
    p.ptr(7, 96);
    p.reg(6, 0x4000);
    p.reg(2, 0x33); // CPU RW plus GPU RW, explicitly requested by the guest.
    p.reg(1, 0);
    call(p, 4, false); // KernelMapFlexibleMemory in its own exact import scope.
    p.expect(0);
    p.load(12, 96);
    p.path(6, 0x4000); // Immutable original template remains in image memory.
    gpu_ptr(p, 7, 0x4000);
    p.reg(1, 0x4000);
    p.code.extend([0xf3, 0xa4]); // Copy the original data page to the actual mapping.
}
fn result_at(p: &mut Program, seed: u32, data: i32) {
    for (lane, value) in [
        (0, seed),
        (7, seed + 7),
        (8, 0xa5a5_a5a5),
        (11, 0xa5a5_a5a5),
    ] {
        read_gpu_result(p, data + lane * 4, false);
        p.expect(u64::from(value));
    }
}
fn consumer(stateful: bool) -> Input {
    let mut words = Vec::<u32>::new();
    fn set(words: &mut Vec<u32>, first: u32, values: &[u32]) -> i32 {
        words.push(0xc000_7602 | ((values.len() as u32) << 16));
        words.push(first);
        let at = COMMANDS + words.len() as i32 * 4;
        words.extend(values);
        at
    }
    set(&mut words, 0x207, &[8, 1, 1]);
    let pgm = set(&mut words, 0x20c, &[0, 0]);
    set(&mut words, 0x212, &[0x2c_0000, 10]);
    set(&mut words, 0x228, &[0]);
    let user = set(&mut words, 0x240, &[0, 0, 48, 0, 17]);
    if !stateful {
        words.extend([0xc003_1502, 1, 1, 1, 0x8005]);
    }
    const ROOT: i32 = 0x4300;
    const LEAF: i32 = 0x4280;
    const CHAIN: i32 = 0x42c0;
    const DATA_B: i32 = DATA + 64;
    let mut nested = Vec::new();
    let desc_a = set(&mut nested, 0x240, &[0, 0, 48, 0]) - COMMANDS + ROOT;
    let call_at = ROOT + nested.len() as i32 * 4;
    nested.extend([0xc002_3f02, 0, 0, 0x0f20_0005]);
    let next_seed = set(&mut nested, 0x244, &[23]) - COMMANDS + ROOT;
    let desc_b = set(&mut nested, 0x240, &[0, 0, 48, 0]) - COMMANDS + ROOT;
    let chain_at = ROOT + nested.len() as i32 * 4;
    nested.extend([0xc002_3f02, 0, 0, 0x0f30_0005]);
    nested.extend([0xc000_2d00, 0]); // Chained target discards this unported tail.
    let mut p = Program::new();
    gpu_allocation(&mut p);
    gpu_ptr(&mut p, 0, SHADER);
    p.code.extend([0x48, 0xc1, 0xe8, 8]);
    store(&mut p, pgm, false);
    gpu_ptr(&mut p, 0, SHADER);
    p.code.extend([0x48, 0xc1, 0xe8, 40]);
    store(&mut p, pgm + 4, false);
    gpu_ptr(&mut p, 0, DATA);
    store(&mut p, user, false);
    p.code.extend([0x48, 0xc1, 0xe8, 32]);
    store(&mut p, user + 4, false);
    gpu_ptr(&mut p, 0, COMMANDS);
    store(&mut p, PACKET, true);
    store(&mut p, ADDRESSES, true);
    if stateful {
        p.reg(7, 0x20);
        gpu_ptr(&mut p, 6, PACKET);
        call(&mut p, 2, stateful);
        p.expect(0);
        p.reg(0, 71);
        store(&mut p, user + 16, false);
        p.reg(7, 0x21);
        gpu_ptr(&mut p, 6, PACKET);
        call(&mut p, 2, stateful);
        p.expect(0);
        for (target, data) in [(desc_a, DATA), (desc_b, DATA_B)] {
            gpu_ptr(&mut p, 0, data);
            store(&mut p, target, false);
            p.code.extend([0x48, 0xc1, 0xe8, 32]);
            store(&mut p, target + 4, false);
        }
        for (target, address) in [
            (call_at + 4, LEAF),
            (chain_at + 4, CHAIN),
            (PACKET, ROOT),
            (ADDRESSES, ROOT),
        ] {
            gpu_ptr(&mut p, 0, address);
            store(&mut p, target, true);
        }
        p.reg(0, nested.len() as u64);
        store(&mut p, PACKET + 8, false);
        store(&mut p, SIZES, false);
    }
    let seeds = if stateful {
        [23_u32, 31, 37, 43]
    } else {
        [17, 23, 31, 37]
    };
    for (slot, seed) in seeds.into_iter().enumerate() {
        let slot = slot as i32;
        let queue = if stateful && slot % 2 == 1 {
            0x21
        } else {
            0x20
        };
        p.reg(0, u64::from(seed));
        store(&mut p, if stateful { next_seed } else { user + 16 }, false);
        if stateful {
            p.reg(0, queue);
            store(&mut p, CONTEXT + 4, false);
        }
        match slot {
            0 => {
                gpu_ptr(&mut p, 7, CONTEXT);
                gpu_ptr(&mut p, 6, PACKET);
            }
            1 => {
                gpu_ptr(&mut p, 7, CONTEXT);
                gpu_ptr(&mut p, 6, ADDRESSES);
                gpu_ptr(&mut p, 2, SIZES);
                p.reg(1, 2);
            }
            2 => {
                p.reg(7, queue);
                gpu_ptr(&mut p, 6, PACKET);
            }
            3 => {
                p.reg(7, queue);
                gpu_ptr(&mut p, 6, ADDRESSES);
                gpu_ptr(&mut p, 2, SIZES);
                p.reg(1, 2);
            }
            _ => unreachable!(),
        }
        call(&mut p, slot, stateful);
        p.expect(0);
        if stateful {
            result_at(&mut p, [17, 71, 23, 31][slot as usize], DATA);
            result_at(&mut p, seed, DATA_B);
        } else {
            result_at(&mut p, seed, DATA);
        }
        if slot == 0 {
            // This boundary follows real fenced GPU work; later submissions
            // must still observe the independent retained compute registers.
            call(&mut p, 5, stateful);
            p.expect(0);
        }
    }
    // Null packet and zero-count branches must not read the deliberately invalid
    // other pointers. A nonempty CommandBuffer with a null context is EINVAL.
    p.reg(7, 0x20);
    p.reg(6, 0);
    call(&mut p, 2, stateful);
    p.expect(0);
    p.reg(7, 1);
    p.reg(6, 1);
    p.reg(2, 1);
    p.reg(1, 0);
    call(&mut p, 1, stateful);
    p.expect(0);
    p.reg(7, 0);
    gpu_ptr(&mut p, 6, PACKET);
    call(&mut p, 0, stateful);
    p.expect(I32_INVALID);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    let (code, calls) = (p.code, p.calls);
    assert!(code.len() < 0x2800);
    assert_eq!(calls, if stateful { 11 } else { 9 });
    let mut input = image(
        &[],
        &[
            NIDS[0],
            NIDS[1],
            NIDS[2],
            NIDS[3],
            "IWIBBdTHit4",
            "h9z6+0hEydk",
        ],
    );
    input.bytes.resize(0x5000, 0);
    put(&mut input.bytes, 0x800, &code); // This consumer needs no file-path literals.
    let mut names = b"\0Graphics5Driver\0Graphics5Driver\0".to_vec();
    for (index, nid) in NIDS.iter().enumerate() {
        put(
            &mut input.bytes,
            0x600 + (index + 1) * 24,
            &(names.len() as u32).to_le_bytes(),
        );
        names.extend_from_slice(format!("{nid}#D#C\0").as_bytes());
    }
    let kernel_name = names.len() as u64;
    names.extend_from_slice(b"libkernel\0");
    put(
        &mut input.bytes,
        0x600 + 5 * 24,
        &(names.len() as u32).to_le_bytes(),
    );
    names.extend_from_slice(b"IWIBBdTHit4#F#E\0");
    let agc_name = names.len() as u64;
    names.extend_from_slice(b"libSceAgc\0");
    put(
        &mut input.bytes,
        0x600 + 6 * 24,
        &(names.len() as u32).to_le_bytes(),
    );
    names.extend_from_slice(b"h9z6+0hEydk#H#G\0");
    for (at, tag, value) in [
        (
            0x2a0,
            0x6100_000f_i64,
            kernel_name | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x2b0, 0x6100_0015_i64, kernel_name | (1 << 32) | (5 << 48)),
        (
            0x2c0,
            0x6100_000f_i64,
            agc_name | (1 << 32) | (1 << 40) | (6 << 48),
        ),
        (0x2d0, 0x6100_0015_i64, agc_name | (1 << 32) | (7 << 48)),
        (0x2e0, 0, 0),
    ] {
        put(&mut input.bytes, at, &tag.to_le_bytes());
        put(&mut input.bytes, at + 8, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut input.bytes, at, &(15_u64 * 16).to_le_bytes());
    }
    input.bytes[0x400..0x600].fill(0);
    put(&mut input.bytes, 0x400, &names);
    put(&mut input.bytes, 0x218, &(names.len() as u64).to_le_bytes());
    put(
        &mut input.bytes,
        0x258,
        &(1_u64 | (1 << 32) | (1 << 40) | (2 << 48)).to_le_bytes(),
    );
    put(
        &mut input.bytes,
        0x268,
        &(17_u64 | (1 << 32) | (3 << 48)).to_le_bytes(),
    );
    input.bytes.resize(0x5000, 0);
    {
        put(&mut input.bytes, 96, &0x4000_u64.to_le_bytes());
        for index in 0..6 {
            put(
                &mut input.bytes,
                0x700 + index * 24,
                &(0x3000_u64 + index as u64 * 8).to_le_bytes(),
            );
        }
    }
    put(&mut input.bytes, 56, &3_u16.to_le_bytes());
    let at = 176;
    put(&mut input.bytes, at, &1_u32.to_le_bytes());
    put(&mut input.bytes, at + 4, &6_u32.to_le_bytes());
    for (delta, value) in [
        (8, 0x4000_u64),
        (16, 0x4000),
        (32, 0x1000),
        (40, 0x4000),
        (48, 0x4000),
    ] {
        put(&mut input.bytes, at + delta, &value.to_le_bytes());
    }
    for (index, word) in words.iter().enumerate() {
        put(
            &mut input.bytes,
            COMMANDS as usize + index * 4,
            &word.to_le_bytes(),
        );
    }
    if stateful {
        for (address, stream) in [
            (ROOT, nested.as_slice()),
            (LEAF, &[0xc003_1502, 1, 1, 1, 0x8005]),
            (CHAIN, &[0xc003_1502, 1, 1, 1, 0x8005]),
        ] {
            for (index, word) in stream.iter().enumerate() {
                put(
                    &mut input.bytes,
                    address as usize + index * 4,
                    &word.to_le_bytes(),
                );
            }
        }
        input.bytes[DATA_B as usize..DATA_B as usize + 48].fill(0xa5);
    }
    for (index, word) in [
        (0x25_u32 << 25) | (1 << 17) | 4,
        (0x1a << 25) | (2 << 17) | 130,
        0xe070_1000,
        (128 << 24) | (1 << 8) | 2,
        0xbf81_0000,
    ]
    .iter()
    .enumerate()
    {
        put(
            &mut input.bytes,
            SHADER as usize + index * 4,
            &word.to_le_bytes(),
        );
    }
    put(
        &mut input.bytes,
        CONTEXT as usize + 4,
        &0x20_u32.to_le_bytes(),
    );
    put(
        &mut input.bytes,
        PACKET as usize + 8,
        &(words.len() as u32).to_le_bytes(),
    );
    // The pinned path ignores flags/reserved; they must not become commands.
    put(
        &mut input.bytes,
        PACKET as usize + 12,
        &[0xff, 0xa5, 0x5a, 0x7f],
    );
    put(
        &mut input.bytes,
        SIZES as usize,
        &(words.len() as u32).to_le_bytes(),
    );
    input.bytes[DATA as usize..DATA as usize + 48].fill(0xa5);
    std::fs::write(&input.path, &input.bytes).unwrap();
    input
}
#[test]
fn native_guest_graphics_submit_imports_execute_four_gpu_jobs_and_handle_null_branches() {
    let original = consumer(false);
    let (code, fields, text) = probe(&original);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "9", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}
#[test]
fn native_guest_compute_queues_retain_isolated_state_and_execute_indirect_multi_dispatch() {
    let original = consumer(true);
    let (code, fields, text) = probe(&original);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "11", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_guest_memory_commands_feed_live_indirect_dispatch_and_completion_labels() {
    const STREAM: i32 = 0x5000;
    const ARGS: i32 = 0x6800;
    const SKIPPED: i32 = 0x6840;
    const OUTPUT: i32 = 0x6880;
    const LABEL: i32 = 0x6900;
    let mut input = consumer(false);
    let mut words = Vec::new();
    let mut pointers = Vec::new();
    fn set(words: &mut Vec<u32>, first: u32, values: &[u32]) -> i32 {
        words.extend([0xc000_7602 | ((values.len() as u32) << 16), first]);
        let at = STREAM + words.len() as i32 * 4;
        words.extend(values);
        at
    }
    fn ptr(words: &mut Vec<u32>, pointers: &mut Vec<(i32, i32)>, address: i32) {
        pointers.push((STREAM + words.len() as i32 * 4, address));
        words.extend([0, 0]);
    }
    fn descriptor(
        words: &mut Vec<u32>,
        pointers: &mut Vec<(i32, i32)>,
        address: i32,
        length: u32,
        seed: u32,
    ) {
        let at = set(words, 0x240, &[0, 0, length, 0, seed]);
        pointers.push((at, address));
    }
    set(&mut words, 0x207, &[3, 1, 1]);
    let pgm = set(&mut words, 0x20c, &[0, 0]);
    set(&mut words, 0x212, &[0x2c_0000, 10]);
    set(&mut words, 0x228, &[0]);
    descriptor(&mut words, &mut pointers, ARGS, 12, 0);
    words.extend([0xc003_1502, 1, 1, 1, 0x8005]); // Actual GPU produces {0,1,2}.
    set(&mut words, 0x207, &[8, 1, 1]);
    descriptor(&mut words, &mut pointers, SKIPPED, 48, 99);
    words.push(0xc002_1600);
    ptr(&mut words, &mut pointers, ARGS);
    words.push(0x8005);
    // The indirect dispatch must observe GPU-produced zero X, not the original
    // {1,1,1} argument bytes captured before execution.
    words.extend([0xc005_3700, 0x100]);
    ptr(&mut words, &mut pointers, ARGS);
    words.extend([1, 1, 1]);
    words.extend([0xc002_1102, 1]);
    ptr(&mut words, &mut pointers, ARGS - 8);
    descriptor(&mut words, &mut pointers, OUTPUT, 48, 23);
    words.extend([0xc001_1600, 8, 0x8005]);
    words.extend([0xc005_3700, 0x10100]);
    ptr(&mut words, &mut pointers, LABEL);
    words.extend([7, 11, 17]);
    words.extend([0xc005_3c00, 0x13]);
    ptr(&mut words, &mut pointers, LABEL);
    words.extend([17, 0xffff_ffff, 0]);
    for (address, selection, value) in [
        (LABEL + 4, 0x2000_0000, 66_u64),
        (LABEL + 8, 0x4000_0000, 0x0123_4567_89ab_cdef),
    ] {
        words.extend([0xc006_1060, 0x62f, selection]);
        ptr(&mut words, &mut pointers, address);
        words.extend([value as u32, (value >> 32) as u32, 0]);
    }
    words.extend([0xc007_9300, 0x113]);
    ptr(&mut words, &mut pointers, LABEL + 8);
    words.extend([0x89ab_cdef, 0x0123_4567, 0xffff_ffff, 0xffff_ffff, 0]);
    let mut p = Program::new();
    gpu_allocation(&mut p);
    gpu_ptr(&mut p, 0, SHADER);
    p.code.extend([0x48, 0xc1, 0xe8, 8]);
    store(&mut p, pgm, false);
    gpu_ptr(&mut p, 0, SHADER);
    p.code.extend([0x48, 0xc1, 0xe8, 40]);
    store(&mut p, pgm + 4, false);
    for (at, address) in pointers {
        gpu_ptr(&mut p, 0, address);
        store(&mut p, at, true);
    }
    gpu_ptr(&mut p, 0, STREAM);
    store(&mut p, PACKET, true);
    p.reg(0, words.len() as u64);
    store(&mut p, PACKET + 8, false);
    p.reg(7, 0x20);
    gpu_ptr(&mut p, 6, PACKET);
    call(&mut p, 2, false);
    p.expect(0);
    for offset in [0, 7 * 4, 8 * 4, 11 * 4] {
        read_gpu_result(&mut p, SKIPPED + offset, false);
        p.expect(0xa5a5_a5a5);
    }
    result_at(&mut p, 23, OUTPUT);
    for (address, expected) in [
        (ARGS, 1_u64),
        (ARGS + 4, 1),
        (ARGS + 8, 1),
        (LABEL, 17),
        (LABEL + 4, 66),
    ] {
        read_gpu_result(&mut p, address, false);
        p.expect(expected);
    }
    read_gpu_result(&mut p, LABEL + 8, true);
    p.expect(0x0123_4567_89ab_cdef);
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 2);
    input.bytes.resize(0x8000, 0);
    put(&mut input.bytes, 176 + 32, &0x4000_u64.to_le_bytes());
    input.bytes[0x800..0x3000].fill(0);
    put(&mut input.bytes, 0x800, &code);
    for (index, word) in words.iter().enumerate() {
        put(
            &mut input.bytes,
            STREAM as usize + index * 4,
            &word.to_le_bytes(),
        );
    }
    for index in 0..3 {
        put(
            &mut input.bytes,
            ARGS as usize + index * 4,
            &1_u32.to_le_bytes(),
        );
    }
    input.bytes[SKIPPED as usize..SKIPPED as usize + 48].fill(0xa5);
    input.bytes[OUTPUT as usize..OUTPUT as usize + 48].fill(0xa5);
    std::fs::write(&input.path, &input.bytes).unwrap();
    let (code, fields, text) = probe(&input);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "2", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}
