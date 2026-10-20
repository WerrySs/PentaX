// SPDX-License-Identifier: GPL-2.0-only
//! Original command streams; these refusals and zero draws never enter Vulkan.
use super::*;
use crate::gpu::driver::{Function, Limits};
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn words(memory: &mut PausedNativeMemory<'_>, address: u64, input: &[u32]) {
    let bytes: Vec<_> = input.iter().flat_map(|word| word.to_le_bytes()).collect();
    memory.write(address, &bytes).unwrap();
}
fn packet(memory: &mut PausedNativeMemory<'_>, address: u64, commands: u64, dwords: u32) {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&commands.to_le_bytes());
    bytes[8..12].copy_from_slice(&dwords.to_le_bytes());
    memory.write(address, &bytes).unwrap();
}

#[test]
fn context_graphics_retains_one_queue_state_but_explicit_acb_and_done_do_not_invent_it() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    memory.write(base + 4, &0_u32.to_le_bytes()).unwrap();
    words(&mut memory, base + 0x100, &[0xc000_2f00, 1]); // NUM_INSTANCES
    packet(&mut memory, base + 0x20, base + 0x100, 2);
    let mut driver = Driver::new(Limits::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert!(!driver.frame.reset_before_graphics);
    assert_eq!(driver.graphics_owner, Some(memory.owner_id()));
    // A different non-ACB selector is the same source graphics processor.
    memory
        .write(base + 4, &0xffff_ffff_u32.to_le_bytes())
        .unwrap();
    words(&mut memory, base + 0x100, &[0xc001_2d00, 0, 2]);
    packet(&mut memory, base + 0x20, base + 0x100, 3);
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(driver.usage.draws, 0);
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0, base + 0x20, 0, 0, 0, 0]),
        Err(Failure::UnsupportedQueue)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::SuspendPoint, [0; 6]),
        Ok(0)
    );
    assert!(matches!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Err(Failure::GraphicsPreparation(draw::Error::Unsupported(
            "unknown NUM_INSTANCES"
        )))
    ));
    assert!(driver.frame.reset_before_graphics);
    assert!(!driver.poisoned);
    assert_eq!(driver.usage.written_bytes, 0);
    // Retained register state is never accepted on a different actual owner.
    let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let mut foreign_memory = foreign.pause().unwrap();
    assert_eq!(
        driver.submit_graphics(
            &mut foreign_memory,
            &[NativeRange {
                address: base + 0x100,
                size: 12
            }]
        ),
        Err(Failure::Memory)
    );
}

#[test]
fn complete_multibuffer_admission_and_live_refetch_precede_state_publication_or_gpu_effects() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    memory.write(base + 4, &0_u32.to_le_bytes()).unwrap();
    memory
        .write(base + 0x40, &(base + 0x100).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x48, &(base + 0x180).to_le_bytes())
        .unwrap();
    words(&mut memory, base + 0x60, &[5, 1]);
    words(
        &mut memory,
        base + 0x100,
        &[0xc000_2f00, 1, 0xc001_2d00, 0, 2],
    );
    words(&mut memory, base + 0x180, &[0]); // unsupported TYPE0, after valid prefix
    memory.write(base + 0x200, &[0xa5; 32]).unwrap();
    let args = [base, base + 0x40, base + 0x60, 2, 0, 0];
    let mut driver = Driver::new(Limits::default());
    assert!(matches!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Err(Failure::GraphicsPreparation(draw::Error::Packet {
            buffer: 1,
            ..
        }))
    ));
    assert!(driver.frame.reset_before_graphics);
    assert_eq!(driver.graphics_owner, None);
    assert!(!driver.poisoned);
    // Mutating the live second stream must be seen; no retained source replay.
    words(&mut memory, base + 0x180, &[0x8000_0000]);
    assert_eq!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Ok(0)
    );
    assert!(!driver.frame.reset_before_graphics);
    let mut actual = [0; 32];
    memory.read(base + 0x200, &mut actual).unwrap();
    assert_eq!(actual, [0xa5; 32]);
    assert_eq!(driver.usage.draws, 0);
    assert_eq!(driver.usage.written_bytes, 0);
    // Real nonzero draw without required resources refuses before the GPU;
    // publication/reset do not advance. A terminal driver never retries it.
    driver
        .submit(&mut memory, Function::SuspendPoint, [0; 6])
        .unwrap();
    words(
        &mut memory,
        base + 0x100,
        &[0xc000_2f00, 1, 0xc001_2d00, 3, 2],
    );
    assert!(matches!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Err(Failure::GraphicsPreparation(_))
    ));
    assert!(driver.frame.reset_before_graphics);
    assert!(!driver.poisoned);
    driver.poisoned = true;
    let before = driver.usage();
    assert_eq!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Err(Failure::Poisoned)
    );
    assert_eq!(driver.usage(), before);
}
