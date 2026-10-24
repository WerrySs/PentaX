// SPDX-License-Identifier: GPL-2.0-only
//! Original mixed DCB processor consumers. Ordinary cases enter no GPU backend;
//! the explicitly ignored physical case requires actual compute/draw fences.
use super::super::{Function, GraphicsProgress};
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn owner() -> NativeAddressSpace {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x40000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    owner
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn put(memory: &mut PausedNativeMemory<'_>, at: u64, words: &[u32]) -> NativeRange {
    let data = bytes(words);
    memory.write(at, &data).unwrap();
    NativeRange {
        address: at,
        size: data.len() as u64,
    }
}
fn cs(words: &mut Vec<u32>, first: u32, values: &[u32]) {
    words.push(0xc000_7602 | ((values.len() as u32) << 16));
    words.push(first);
    words.extend_from_slice(values);
}
fn zero_dispatch(words: &mut Vec<u32>) {
    words.extend([0xc003_1502, 0, 2, 3, 0x41]);
}
fn write(words: &mut Vec<u32>, at: u64, value: u32) {
    words.extend([0xc003_3700, 0x100, at as u32, (at >> 32) as u32, value]);
}
fn copy(words: &mut Vec<u32>, from: u64, to: u64) {
    words.extend([
        0xc004_4000,
        0x101,
        from as u32,
        (from >> 32) as u32,
        to as u32,
        (to >> 32) as u32,
    ]);
}
fn wait(words: &mut Vec<u32>, at: u64) {
    words.extend([
        0xc005_3c00,
        0x13,
        at as u32,
        (at >> 32) as u32,
        7,
        u32::MAX,
        0,
    ]);
}
fn packet(memory: &mut PausedNativeMemory<'_>, at: u64, range: NativeRange) {
    let mut data = [0; 16];
    data[..8].copy_from_slice(&range.address.to_le_bytes());
    data[8..12].copy_from_slice(&(range.size as u32 / 4).to_le_bytes());
    memory.write(at, &data).unwrap();
}
fn submit(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    descriptor: u64,
    range: NativeRange,
) {
    packet(memory, descriptor, range);
    assert_eq!(
        driver.submit(memory, Function::Dcb, [descriptor, 0, 0, 0, 0, 0]),
        Ok(0)
    );
}
fn pump(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>) {
    for _ in 0..64 {
        if !driver.pump_direct_graphics(memory, None, None).unwrap() {
            return;
        }
    }
    panic!("original mixed DCB did not finish within bounded turns");
}
fn read(memory: &PausedNativeMemory<'_>, at: u64, count: usize) -> Vec<u8> {
    let mut data = vec![0; count];
    memory.read(at, &mut data).unwrap();
    data
}
fn word(memory: &PausedNativeMemory<'_>, at: u64) -> u32 {
    u32::from_le_bytes(read(memory, at, 4).try_into().unwrap())
}
fn expected_compute(
    initial: compute::ComputeState,
    memory: &PausedNativeMemory<'_>,
    range: NativeRange,
) -> compute::ComputeState {
    // The independently existing ACB planner is the reference processor for
    // CS-only synthetic words; it executes no command or backend work here.
    compute::plan_commands(
        &initial,
        compute::CommandRange {
            address: range.address,
            size: range.size,
        },
        &crate::gpu::Memory(memory),
        &compute::CommandLimits::default(),
    )
    .unwrap()
    .final_state()
}
fn seed_acb(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>, base: u64) {
    let mut stream = Vec::new();
    cs(&mut stream, 0x215, &[0x1357_2468]);
    let range = put(memory, base + 0x7000, &stream);
    packet(memory, base + 0x200, range);
    assert_eq!(
        driver.submit(memory, Function::Acb, [0x27, base + 0x200, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_eq!(driver.pending_compute_queues(), 0);
    assert_ne!(driver.queues[7], compute::ComputeState::for_new_queue());
}

#[test]
fn mixed_dcb_zero_dispatches_retire_separate_cs_state_without_touching_any_acb_queue() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    seed_acb(&mut driver, &mut memory, base);
    let acb = driver.queues;
    let out = base + 0x30000;
    memory.write(out, &[0xa5; 12]).unwrap();
    let mut stream = Vec::new();
    cs(&mut stream, 0x215, &[17]);
    cs(&mut stream, 0x22a, &[31]);
    zero_dispatch(&mut stream);
    stream.extend([0xc003_1500, 1, 0, 5, 0x8041]); // Both admitted widths, no shader needed.
    write(&mut stream, out + 4, 0x2468_1357);
    let range = put(&mut memory, base + 0x1000, &stream);
    let expected = expected_compute(compute::ComputeState::for_new_queue(), &memory, range);
    assert_ne!(expected, compute::ComputeState::for_new_queue());
    submit(&mut driver, &mut memory, base + 0x100, range);
    pump(&mut driver, &mut memory);
    assert_eq!(driver.graphics_compute, expected);
    assert_eq!(driver.queues, acb);
    assert_eq!(driver.usage.dispatches, 2);
    assert_eq!(driver.usage.invocations, 0);
    assert_eq!(driver.usage.draws, 0);
    assert_eq!(driver.usage.written_bytes, 4);
    assert_eq!(
        read(&memory, out, 12),
        bytes(&[0xa5a5_a5a5, 0x2468_1357, 0xa5a5_a5a5])
    );
}

#[test]
fn mixed_dcb_wait_fifo_selects_completed_cs_at_head_and_never_replays_sealed_prefix() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    let acb = driver.queues;
    let label = base + 0x30000;
    let gate = label + 8;
    let output = label + 16;
    let tail = label + 24;
    memory.write(label, &[0; 32]).unwrap();
    memory
        .write(label + 4, &0x1357_9bdfu32.to_le_bytes())
        .unwrap();
    memory
        .write(output + 4, &0x2468_ace0u32.to_le_bytes())
        .unwrap();
    let mut first = Vec::new();
    cs(&mut first, 0x215, &[17]);
    write(&mut first, label, 17);
    wait(&mut first, gate);
    cs(&mut first, 0x22a, &[31]);
    zero_dispatch(&mut first);
    copy(&mut first, label, output);
    let one = put(&mut memory, base + 0x1000, &first);
    let first_state = expected_compute(compute::ComputeState::for_new_queue(), &memory, one);
    submit(&mut driver, &mut memory, base + 0x100, one);
    assert!(
        driver
            .pump_direct_graphics(&mut memory, None, None)
            .unwrap()
    );
    assert_eq!(word(&memory, label), 17);
    assert_eq!(word(&memory, output), 0);
    assert_eq!(
        driver.graphics_compute,
        compute::ComputeState::for_new_queue()
    );
    let mut second = Vec::new();
    cs(&mut second, 0x22a, &[41]);
    zero_dispatch(&mut second);
    write(&mut second, tail, 41);
    let two = put(&mut memory, base + 0x2000, &second);
    let expected = expected_compute(first_state, &memory, two);
    submit(&mut driver, &mut memory, base + 0x120, two);
    put(&mut memory, one.address, &vec![0x8000_0000; first.len()]);
    memory.write(label, &99u32.to_le_bytes()).unwrap();
    assert!(
        driver
            .pump_direct_graphics(&mut memory, None, None)
            .unwrap()
    );
    assert_eq!((word(&memory, output), word(&memory, tail)), (0, 0));
    memory.write(gate, &7u32.to_le_bytes()).unwrap();
    pump(&mut driver, &mut memory);
    assert_eq!(driver.graphics_compute, expected);
    assert_eq!(driver.queues, acb);
    assert_eq!(
        (
            word(&memory, label),
            word(&memory, output),
            word(&memory, tail)
        ),
        (99, 99, 41)
    );
    assert_eq!(
        (word(&memory, label + 4), word(&memory, output + 4)),
        (0x1357_9bdf, 0x2468_ace0)
    );
    assert_eq!(driver.usage.dispatches, 2);
    assert_eq!(driver.usage.memory_written_bytes, 12);
    assert_eq!(driver.pending_direct_graphics(), 0);
    assert!(!driver.poisoned);
}

#[test]
fn mixed_dcb_context_clear_preserves_cs_but_real_suspend_resets_only_the_next_graphics_head() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    seed_acb(&mut driver, &mut memory, base);
    let acb = driver.queues;
    let mut reference = Vec::new();
    cs(&mut reference, 0x215, &[99]);
    let reference_range = put(&mut memory, base + 0xa000, &reference);
    let expected = expected_compute(
        compute::ComputeState::for_new_queue(),
        &memory,
        reference_range,
    );
    let mut stream = reference;
    zero_dispatch(&mut stream);
    stream.extend([0xc000_2f00, 7, 0xc000_1200, 0]); // NUM_INSTANCES then ContextClear.
    let range = put(&mut memory, base + 0x1000, &stream);
    submit(&mut driver, &mut memory, base + 0x100, range);
    let GraphicsProgress::Waiting(boundary) = driver
        .queue_graphics_suspend(&mut memory, None, None)
        .unwrap()
    else {
        panic!("SuspendPoint overtook the accepted mixed DCB");
    };
    pump(&mut driver, &mut memory);
    assert_eq!(driver.graphics_compute, expected);
    assert_ne!(driver.graphics, draw::GraphicsState::for_new_processor());
    assert_eq!(driver.queues, acb);
    assert!(!driver.frame.reset_before_graphics);
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, boundary, None, None)
            .unwrap(),
        GraphicsProgress::Complete(0)
    ));
    assert_eq!(driver.frame.boundaries, 1);
    assert!(driver.frame.reset_before_graphics);
    assert_eq!(driver.graphics_compute, expected); // Boundary does not rewrite retired state.
    let next = put(&mut memory, base + 0x2000, &[0x8000_0000]);
    submit(&mut driver, &mut memory, base + 0x120, next);
    pump(&mut driver, &mut memory);
    assert_eq!(
        driver.graphics_compute,
        compute::ComputeState::for_new_queue()
    );
    assert_eq!(driver.graphics, draw::GraphicsState::for_new_processor());
    assert_eq!(driver.queues, acb);
    assert!(!driver.frame.reset_before_graphics);
    assert_eq!(driver.usage.dispatches, 1);
    assert_eq!(driver.usage.written_bytes, 0);
}

#[test]
fn mixed_dcb_foreign_owner_late_gpu_rights_and_dispatch_cap_refuse_before_state_or_output_effects()
{
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut driver = Driver::new(Default::default());
    let (saved, acb) = {
        let mut memory = space.pause().unwrap();
        let mut stream = Vec::new();
        cs(&mut stream, 0x215, &[17]);
        let range = put(&mut memory, base + 0x1000, &stream);
        submit(&mut driver, &mut memory, base + 0x100, range);
        pump(&mut driver, &mut memory);
        let saved = driver.graphics_compute;
        let acb = driver.queues;
        memory.write(base + 0x30000, &[0xa5; 12]).unwrap();
        let mut next = Vec::new();
        cs(&mut next, 0x215, &[99]);
        zero_dispatch(&mut next);
        write(&mut next, base + 0x30004, 0x2468_1357);
        let range = put(&mut memory, base + 0x12000, &next);
        submit(&mut driver, &mut memory, base + 0x120, range);
        (saved, acb)
    };
    let mut foreign = owner();
    let other = foreign.reserved_ranges().next().unwrap().address;
    {
        let mut memory = foreign.pause().unwrap();
        let range = put(&mut memory, other + 0x1000, &[0x8000_0000]);
        packet(&mut memory, other + 0x100, range);
        assert_eq!(
            driver.submit(&mut memory, Function::Dcb, [other + 0x100, 0, 0, 0, 0, 0]),
            Err(Failure::Memory)
        );
        assert_eq!(driver.graphics_compute, saved);
        assert_eq!(driver.queues, acb);
        assert!(!driver.poisoned);
        assert_eq!(driver.pending_direct_graphics(), 1);
    }
    // Sealing bytes is not permanent read authority: revoke the command page's GPU right.
    space
        .protect(base + 0x10000, 0x10000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    assert_eq!(
        driver.pump_direct_graphics(&mut memory, None, None),
        Err(Failure::Memory)
    );
    assert_eq!(driver.graphics_compute, saved);
    assert_eq!(driver.queues, acb);
    assert_eq!(read(&memory, base + 0x30000, 12), [0xa5; 12]);
    assert_eq!(driver.usage.dispatches, 0);
    assert_eq!(driver.usage.written_bytes, 0);
    assert!(driver.poisoned); // Accepted asynchronous failures are terminal, even before an effect.
    driver.revoke_graphics_owner(memory.owner_id()).unwrap();
    assert_eq!(driver.pending_direct_graphics(), 0);
    // A separate fresh driver proves zero-group dispatches still consume the existing cap.
    let mut limited = Driver::new(super::super::Limits {
        max_dispatches: 0,
        ..Default::default()
    });
    let mut stream = Vec::new();
    cs(&mut stream, 0x215, &[99]);
    zero_dispatch(&mut stream);
    write(&mut stream, base + 0x30004, 0x2468_1357);
    let range = put(&mut memory, base + 0x3000, &stream);
    submit(&mut limited, &mut memory, base + 0x140, range);
    assert_eq!(
        limited.pump_direct_graphics(&mut memory, None, None),
        Err(Failure::GraphicsPreparation(draw::Error::Limit(
            "command dispatches"
        )))
    );
    assert_eq!(
        limited.graphics_compute,
        compute::ComputeState::for_new_queue()
    );
    assert_eq!(limited.usage.dispatches, 0);
    assert_eq!(limited.usage.written_bytes, 0);
    assert_eq!(read(&memory, base + 0x30000, 12), [0xa5; 12]);
}

// Original project AMD shader and fixed 64x64 state helpers, specialized here
// only for a computed readonly VS resource and one draw. No host shader exists.
fn unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e00_0000 | (dst << 17) | (op << 9) | src
}
fn binary(op: u32, dst: u32, src: u32, vector: u32) -> u32 {
    (op << 25) | (dst << 17) | (vector << 9) | src
}
fn export(target: u32, registers: [u32; 4]) -> [u32; 2] {
    [
        0xf800_080f | (target << 4),
        registers[0] | (registers[1] << 8) | (registers[2] << 16) | (registers[3] << 24),
    ]
}
fn vertex_shader() -> Vec<u32> {
    // VertexIndex is the source ABI's v5. The three positions are:
    // (-.5 + s0, -.5), (.5 + s0, -.5), (-.5 + s0, .5).
    // Inline float operands240/241 mean +.5/-.5. Source SGPR0 is a real
    // per-draw horizontal offset, carried by the vertex push-data manifest.
    let mut words = vec![
        binary(0x1a, 0, 129, 5), // V_LSHLREV_B32 v0, 1, v5.
        binary(0x1b, 0, 130, 0), // V_AND_B32 v0, 2, v0.
        binary(0x1b, 1, 130, 5), // V_AND_B32 v1, 2, v5.
        unary(6, 0, 256),        // V_CVT_F32_U32 v0, v0.
        unary(6, 1, 257),        // V_CVT_F32_U32 v1, v1.
        binary(8, 0, 240, 0),    // V_MUL_F32 v0, .5, v0.
        binary(8, 1, 240, 1),
        binary(3, 0, 241, 0), // V_ADD_F32 v0, -.5, v0.
        binary(3, 1, 241, 1),
        binary(3, 0, 0, 0), // V_ADD_F32 v0, s0, v0.
        unary(1, 2, 128),   // z=0.
        unary(1, 3, 242),   // w=1.
    ];
    words.extend(export(12, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}
fn pixel_shader() -> Vec<u32> {
    // Actual PS user SGPRs0..2 supply RGB; alpha is one. Neither the compiler
    // nor this test supplies a host-language shader or hard-coded GPU result.
    let mut words = vec![
        unary(1, 0, 0),
        unary(1, 1, 1),
        unary(1, 2, 2),
        unary(1, 3, 242),
    ];
    words.extend(export(0, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}
fn set(words: &mut Vec<u32>, opcode: u32, register: u32, values: &[u32]) {
    assert!(!values.is_empty() && values.len() < 16384);
    words.push(0xc000_0000 | ((values.len() as u32) << 16) | (opcode << 8));
    words.push(register);
    words.extend_from_slice(values);
}
fn draw_commands(vertex: u64, pixel: u64, target: u64, resource: u64) -> Vec<u32> {
    let mut words = Vec::new();
    for register in [
        0x80, 0x1b1, 0x1b3, 0x1b4, 0x1b8, 0x1c4, 0x1e0, 0x200, 0x203, 0x205, 0x207, 0x293, 0x2f8,
        0x31b, 0x31d, 0x31e, 0x31f, 0x321, 0x325, 0x398, 0x3a0, 0x3a8,
    ] {
        set(&mut words, 0x69, register, &[0]);
    }
    for (register, value) in [
        (0x8e, 15),
        (0x8f, 15),
        (0x1b6, 0x8000),
        (0x1c3, 4),
        (0x1c5, 9),
        (0x202, 0x00cc_0010),
        (0x204, 1 << 19),
        (0x206, 0x3f),
        (0x292, 2),
        (0x2d5, 0x0040_0000),
        (0x318, (target >> 8) as u32),
        (0x31c, (10 << 2) | (1 << 15)),
        (0x390, (target >> 40) as u32),
        (0x3b0, (63 << 14) | 63),
        (0x3b8, 1 << 24),
    ] {
        set(&mut words, 0x69, register, &[value]);
    }
    for (left, right, origin) in [
        (0x0c, 0x0d, 0),
        (0x81, 0x82, 0x8000_0000),
        (0x90, 0x91, 0x8000_0000),
        (0x94, 0x95, 0x8000_0000),
    ] {
        set(&mut words, 0x69, left, &[origin]);
        set(&mut words, 0x69, right, &[64 | (64 << 16)]);
    }
    set(
        &mut words,
        0x69,
        0x10f,
        &[
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            1_f32.to_bits(),
            0,
        ],
    );
    set(
        &mut words,
        0x76,
        0xc8,
        &[(vertex >> 8) as u32, (vertex >> 40) as u32],
    );
    set(&mut words, 0x76, 0x88, &[0, 0, 0x602c_0000, 0x0003_0002, 0]);
    set(
        &mut words,
        0x76,
        0x08,
        &[
            (pixel >> 8) as u32,
            (pixel >> 40) as u32,
            0x002c_0000,
            6,
            1_f32.to_bits(),
            0,
            0,
        ],
    );
    set(&mut words, 0x79, 0x242, &[4]);
    set(&mut words, 0x79, 0x24a, &[0]);
    set(&mut words, 0x76, 0x8a, &[0x602c_0002, 0x0003_0010]);
    set(
        &mut words,
        0x76,
        0x90,
        &[resource as u32, (resource >> 32) as u32, 16, 0],
    );
    words.extend([0xc000_2f00, 1, 0xc001_2d00, 3, 2]);
    words
}

#[test]
#[ignore = "requires an explicit real Vulkan compute and draw device run"]
fn actual_mixed_dcb_compute_writeback_copy_and_live_vertex_read_shift_real_draw_pixels() {
    const PAGE: usize = 65536;
    const TARGET_BYTES: usize = 64 * 64 * 4;
    const SENTINEL: [u8; 4] = [17, 31, 47, 63];
    const RED: [u8; 4] = [255, 0, 0, 255];
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let command = base + 0x1000;
    let compute_code = base + 0x10100;
    let vertex_code = base + 0x11100;
    let pixel_code = base + 0x12100;
    let produced = base + 0x20100;
    let vertex_resource = base + 0x20200;
    let marker = base + 0x20300;
    let target = base + 0x30100;
    let mut expected = vec![0xa5; 4 * PAGE];
    // Compute writes exactly one real float .5 through a 16-byte storage buffer.
    // The remaining words must survive actual whole-buffer GPU writeback.
    let compute_shader = [
        unary(1, 1, 255),
        0.5_f32.to_bits(),
        unary(1, 2, 128),
        0xe070_1000,
        (128 << 24) | (1 << 8) | 2,
        0xbf81_0000,
    ];
    let mut vertex = vertex_shader();
    vertex[9] = binary(3, 0, 260, 0); // v0 += loaded v4, not descriptor words or an SGPR constant.
    let mut vertex_shader = vec![0xe030_0000, 0x8001_0400]; // Raw DWORD -> v4 using s[4..7].
    vertex_shader.extend(vertex);
    let pixel_shader = pixel_shader();
    for (at, words) in [
        (compute_code, compute_shader.as_slice()),
        (vertex_code, vertex_shader.as_slice()),
        (pixel_code, pixel_shader.as_slice()),
    ] {
        let data = bytes(words);
        let offset = (at - base) as usize;
        expected[offset..offset + data.len()].copy_from_slice(&data);
    }
    expected[0x20100..0x20104].fill(0);
    expected[0x20200..0x20210].fill(0xc3);
    expected[0x20200..0x20204].fill(0); // Missing compute/COPY leaves zero shift, a different triangle.
    expected[0x30100..0x30100 + TARGET_BYTES].copy_from_slice(&SENTINEL.repeat(64 * 64));
    let mut stream = Vec::new();
    write(&mut stream, marker, 0x1357_2468);
    cs(&mut stream, 0x204, &[0, 0, 0]);
    cs(&mut stream, 0x207, &[1, 1, 1]);
    cs(
        &mut stream,
        0x20c,
        &[(compute_code >> 8) as u32, (compute_code >> 40) as u32],
    );
    cs(&mut stream, 0x212, &[0x002c_0000, 8]); // Strict existing R1; four user SGPRs, no thread IDs.
    cs(&mut stream, 0x228, &[0]);
    cs(
        &mut stream,
        0x240,
        &[produced as u32, (produced >> 32) as u32, 16, 0],
    );
    stream.extend([0xc003_1502, 1, 1, 1, 0x41]); // Actual nonzero wave64 guest dispatch.
    copy(&mut stream, produced, vertex_resource);
    stream.extend(draw_commands(
        vertex_code,
        pixel_code,
        target,
        vertex_resource,
    ));
    write(&mut stream, marker + 8, 0x2468_1357);
    let encoded = bytes(&stream);
    assert!(encoded.len() < 0x3000);
    expected[0x1000..0x1000 + encoded.len()].copy_from_slice(&encoded);
    expected[0x100..0x110].fill(0);
    expected[0x100..0x108].copy_from_slice(&command.to_le_bytes());
    expected[0x108..0x10c].copy_from_slice(&(stream.len() as u32).to_le_bytes());
    memory.write(base, &expected).unwrap();
    let mut driver = Driver::new(Default::default());
    let acb = driver.queues;
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [base + 0x100, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    pump(&mut driver, &mut memory);
    assert_eq!(driver.pending_direct_graphics(), 0);
    assert_eq!(driver.pending_compute_queues(), 0);
    assert_eq!(driver.queues, acb);
    assert!(!driver.poisoned);
    assert_eq!(driver.usage.dispatches, 1);
    assert_eq!(driver.usage.invocations, 1);
    assert_eq!(driver.usage.draws, 1);
    assert_eq!(driver.usage.vertices, 3);
    assert_eq!(driver.usage.memory_written_bytes, 12);
    assert_eq!(driver.usage.written_bytes, (TARGET_BYTES + 16 + 12) as u64);
    assert_eq!(word(&memory, produced), 0.5_f32.to_bits());
    assert_eq!(word(&memory, vertex_resource), 0.5_f32.to_bits());
    assert_eq!(word(&memory, marker), 0x1357_2468);
    assert_eq!(word(&memory, marker + 8), 0x2468_1357); // Reached only after real draw fence/writeback.
    let image = read(&memory, target, TARGET_BYTES);
    let pixel = |x: usize, y: usize| &image[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
    // The .5 offset shifts (-.5,-.5),(.5,-.5),(-.5,.5) right16 pixels:
    // (32,16),(64,16),(32,48). These interior/exterior anchors distinguish
    // zero/default offsets, no copy, stale early resource reads and no draw.
    assert_eq!(pixel(56, 20), RED);
    assert_eq!(pixel(40, 24), RED);
    assert_eq!(pixel(24, 24), SENTINEL);
    assert_eq!(pixel(0, 0), SENTINEL);
    assert_eq!(pixel(56, 56), SENTINEL);
    assert!(
        image
            .chunks_exact(4)
            .all(|pixel| pixel == SENTINEL || pixel == RED)
    );
    expected[0x20100..0x20104].copy_from_slice(&0.5_f32.to_bits().to_le_bytes());
    expected[0x20200..0x20204].copy_from_slice(&0.5_f32.to_bits().to_le_bytes());
    expected[0x20300..0x20304].copy_from_slice(&0x1357_2468u32.to_le_bytes());
    expected[0x20308..0x2030c].copy_from_slice(&0x2468_1357u32.to_le_bytes());
    expected[0x30100..0x30100 + TARGET_BYTES].copy_from_slice(&image);
    // All other four-page bytes, including code, commands, packet, source
    // padding, target padding and every canary, must remain bit-for-bit intact.
    assert_eq!(read(&memory, base, 4 * PAGE), expected);
    println!(
        "mixed_dcb_gpu_available=true mixed_dcb_gpu_dispatches=1 mixed_dcb_gpu_draws=1 mixed_dcb_gpu_computed_vertex_input=true mixed_dcb_gpu_guards=true"
    );
}
