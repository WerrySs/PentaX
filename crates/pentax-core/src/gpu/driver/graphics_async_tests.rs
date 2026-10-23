// SPDX-License-Identifier: GPL-2.0-only
//! Original owned direct DCB consumers; no shader, renderer or ready setter.
use super::super::Function;
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn owner() -> NativeAddressSpace {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x20000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    owner
}
fn words(memory: &mut PausedNativeMemory<'_>, at: u64, words: &[u32]) -> NativeRange {
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    memory.write(at, &bytes).unwrap();
    NativeRange {
        address: at,
        size: bytes.len() as u64,
    }
}
fn write(at: u64, value: u32) -> Vec<u32> {
    vec![0xc003_3700, 0x100, at as u32, (at >> 32) as u32, value]
}
fn wait(at: u64) -> Vec<u32> {
    vec![
        0xc005_3c00,
        0x13,
        at as u32,
        (at >> 32) as u32,
        7,
        u32::MAX,
        0,
    ]
}
fn packet(memory: &mut PausedNativeMemory<'_>, at: u64, range: NativeRange) {
    let mut bytes = [0xa5; 16];
    bytes[..8].copy_from_slice(&range.address.to_le_bytes());
    bytes[8..12].copy_from_slice(&(range.size as u32 / 4).to_le_bytes());
    memory.write(at, &bytes).unwrap();
}
fn read(memory: &PausedNativeMemory<'_>, at: u64) -> u32 {
    let mut bytes = [0; 4];
    memory.read(at, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
fn pump(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>) {
    for _ in 0..64 {
        if !driver.pump_direct_graphics(memory, None, None).unwrap() {
            return;
        }
    }
    panic!("original direct DCB queue did not finish");
}

#[test]
fn direct_dcb_seals_commands_returns_before_wait_and_serializes_legacy_owned_tickets() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let label = base + 0x8000;
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
    let mut first = write(label, 17);
    first.extend(wait(gate));
    first.extend([
        0xc004_4000,
        0x101,
        label as u32,
        (label >> 32) as u32,
        output as u32,
        (output >> 32) as u32,
    ]);
    let root = words(&mut memory, base + 0x1000, &first);
    packet(&mut memory, base + 0x100, root);
    let mut driver = Driver::new(Default::default());
    // There is no context pointer: the ignored second argument is unmapped.
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::Dcb,
            [base + 0x100, u64::MAX, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(driver.pending_direct_graphics(), 1);
    assert!(
        driver
            .pump_direct_graphics(&mut memory, None, None)
            .unwrap()
    );
    assert_eq!(read(&memory, label), 17);
    assert_eq!(read(&memory, output), 0);
    words(&mut memory, root.address, &[0x8000_0000; 18]);
    memory.write(label, &99u32.to_le_bytes()).unwrap();
    let legacy = words(&mut memory, base + 0x2000, &write(tail, 31));
    let GraphicsProgress::Waiting(legacy_ticket) = driver
        .begin_graphics_execution(&mut memory, &[legacy], None, None)
        .unwrap()
    else {
        panic!("legacy FIFO ticket missing");
    };
    assert!(
        !driver
            .graphics_ticket_ready(&memory, &legacy_ticket)
            .unwrap()
    );
    let later = words(&mut memory, base + 0x3000, &write(tail + 4, 41));
    packet(&mut memory, base + 0x120, later);
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [base + 0x120, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    assert!(
        driver
            .pump_direct_graphics(&mut memory, None, None)
            .unwrap()
    );
    assert_eq!((read(&memory, tail), read(&memory, tail + 4)), (0, 0));
    memory.write(gate, &7u32.to_le_bytes()).unwrap();
    assert!(
        driver
            .pump_direct_graphics(&mut memory, None, None)
            .unwrap()
    );
    assert_eq!((read(&memory, label), read(&memory, output)), (99, 99));
    // The background pump cannot consume a CPU-owned legacy admission.
    assert!(
        driver
            .pump_direct_graphics(&mut memory, None, None)
            .unwrap()
    );
    assert_eq!((read(&memory, tail), read(&memory, tail + 4)), (0, 0));
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, legacy_ticket, None, None)
            .unwrap(),
        GraphicsProgress::Complete(0)
    ));
    assert_eq!((read(&memory, tail), read(&memory, tail + 4)), (31, 0));
    pump(&mut driver, &mut memory);
    assert_eq!((read(&memory, tail), read(&memory, tail + 4)), (31, 41));
    assert_eq!(read(&memory, label + 4), 0x1357_9bdf);
    assert_eq!(read(&memory, output + 4), 0x2468_ace0);
    assert_eq!(driver.usage.memory_written_bytes, 16);
    assert_eq!(driver.pending_direct_graphics(), 0);
    assert!(!driver.poisoned);
}

#[test]
fn direct_dcb_zero_null_owner_and_deferred_prefix_failures_keep_real_authority() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [0; 6]),
        Err(Failure::Memory)
    );
    assert!(!driver.poisoned);
    packet(
        &mut memory,
        base + 0x100,
        NativeRange {
            address: u64::MAX,
            size: 0,
        },
    );
    driver.limits.max_batch_buffers = 0;
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [base + 0x100, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_eq!(driver.graphics_admissions.capacity(), 0);
    assert_eq!(driver.pending_direct_graphics(), 0);
    assert_eq!(driver.usage.command_buffers, 0);
    let mut driver = Driver::new(Default::default());
    let label = base + 0x8000;
    let mut stream = write(label, 17);
    stream.extend(write(base + 0x30000, 19));
    let root = words(&mut memory, base + 0x1000, &stream);
    packet(&mut memory, base + 0x100, root);
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [base + 0x100, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    let mut foreign = owner();
    let mut other = foreign.pause().unwrap();
    assert_eq!(
        driver.pump_direct_graphics(&mut other, None, None),
        Err(Failure::Memory)
    );
    assert!(!driver.poisoned);
    assert_eq!(
        driver.pump_direct_graphics(&mut memory, None, None),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, label), 17);
    assert!(driver.poisoned);
    memory.write(label, &99u32.to_le_bytes()).unwrap();
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [base + 0x100, 0, 0, 0, 0, 0]),
        Err(Failure::Poisoned)
    );
    assert_eq!(read(&memory, label), 99);
    driver.revoke_graphics_owner(memory.owner_id()).unwrap();
    assert_eq!(driver.pending_direct_graphics(), 0);
    assert!(driver.graphics_async_ticket.is_none());
    assert_eq!(driver.graphics_retained_bytes().unwrap(), 0);
    assert!(driver.poisoned);

    // Accepted work remains owned even if the later head-preparation budget
    // fails before an execution cursor is published or any write retires.
    let mut driver = Driver::new(Default::default());
    memory.write(label, &0u32.to_le_bytes()).unwrap();
    let root = words(&mut memory, base + 0x1000, &write(label, 23));
    packet(&mut memory, base + 0x100, root);
    assert_eq!(
        driver.submit(&mut memory, Function::Dcb, [base + 0x100, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    driver.limits.max_preparation_work =
        driver.usage.preparation_work + 65 + size_of::<GraphicsAdmission>() as u64;
    assert_eq!(
        driver.pump_direct_graphics(&mut memory, None, None),
        Err(Failure::Limit)
    );
    assert!(
        driver.graphics_admissions.is_empty(),
        "head must be popped before planning fails"
    );
    assert!(driver.graphics_execution.is_none());
    assert_eq!(read(&memory, label), 0);
    assert!(driver.has_pending_graphics_continuation());
    assert_eq!(driver.pending_direct_graphics(), 1);
    assert_eq!(
        driver.revoke_graphics_owner(other.owner_id()),
        Err(Failure::Memory)
    );
    driver.revoke_graphics_owner(memory.owner_id()).unwrap();
    assert!(!driver.has_pending_graphics_continuation());
    assert_eq!(driver.pending_direct_graphics(), 0);
    assert_eq!(driver.graphics_retained_bytes().unwrap(), 0);
}
