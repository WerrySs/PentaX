// SPDX-License-Identifier: GPL-2.0-only
//! Original owned command/shader/data fixtures; no commercial bytes or host SPIR-V.
use super::*;
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};
use std::cell::Cell;

const PAGE: usize = 0x4000;
const CANARY: u8 = 0xa5;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alias {
    None,
    Resource,
    Command,
}
struct Fixture {
    owner: NativeAddressSpace,
    root: NativeRange,
    data: [u64; 4],
    command_page: u64,
    code_page: u64,
    command_image: Vec<u8>,
    code_image: Vec<u8>,
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn stores() -> Vec<u32> {
    let mut words = vec![0x7e04_0280]; // v2=0 byte address.
    for (index, user) in [12u32, 0, 8, 4].into_iter().enumerate() {
        words.extend([
            0x7e02_0280 | (index as u32 + 1),
            0xe070_1000,
            (128 << 24) | ((user / 4) << 16) | (1 << 8) | 2,
        ]);
    }
    words.push(0xbf81_0000);
    words
}
fn copy() -> Vec<u32> {
    vec![
        0x7e04_0280,
        0xe030_1000,
        (128 << 24) | (2 << 16) | (1 << 8) | 2,
        0xe070_1000,
        (128 << 24) | (1 << 16) | (1 << 8) | 2,
        0xbf81_0000,
    ]
}
fn readonly() -> Vec<u32> {
    vec![
        0xf420_0404,
        125 << 25,
        0xbf8c_007f,
        0xbf06_8010,
        0xbf84_0001,
        0xbf80_0000,
        0xbf81_0000,
    ]
}
fn commands(code: u64, descriptors: &[(u32, u64)], users: u32, length: u32) -> Vec<u8> {
    let mut words = Vec::new();
    let mut set = |first: u32, values: &[u32]| {
        words.push(0xc000_7600 | (values.len() as u32) << 16);
        words.push(first);
        words.extend(values);
    };
    set(0x204, &[0, 0, 0]);
    set(0x207, &[1, 1, 1]);
    set(0x20c, &[(code >> 8) as u32, (code >> 40) as u32]);
    set(0x212, &[0x2c0000, users << 1]);
    set(0x228, &[0]);
    for &(user, address) in descriptors {
        set(
            0x240 + user,
            &[address as u32, (address >> 32) as u32, length, 0],
        );
    }
    words.extend([0xc003_1500, 1, 1, 1, 0x41]);
    bytes(&words)
}
fn fixture(code: &[u32], users: u32, descriptor_users: &[u32], alias: Alias) -> Fixture {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x80000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let permissions = NativePermissions::READ_WRITE.with_gpu(true, true);
    owner
        .map_at(base + 0x10000, PAGE as u64, permissions)
        .unwrap();
    if alias != Alias::Command {
        owner.map_at(base, PAGE as u64, permissions).unwrap();
    }
    for index in 0..4 {
        if !(alias == Alias::Resource && index < 2 || alias == Alias::Command && index == 0) {
            owner
                .map_at(
                    base + 0x20000 + index * PAGE as u64,
                    PAGE as u64,
                    permissions,
                )
                .unwrap();
        }
    }
    let mut data = std::array::from_fn(|index| base + 0x20100 + index as u64 * PAGE as u64);
    if alias == Alias::Command {
        data[0] = base + 0x21000;
    }
    let mut memory = owner.pause().unwrap();
    if alias != Alias::None {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::AllocatePhysical {
                        physical: 0,
                        size: PAGE as u64,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
        let views = if alias == Alias::Resource {
            [base + 0x20000, base + 0x24000]
        } else {
            [base, base + 0x20000]
        };
        for address in views {
            memory
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::MapPhysical {
                            address,
                            size: PAGE as u64,
                            physical: 0,
                            permissions,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        }
    }
    let mut command_image = vec![CANARY; PAGE];
    let mut code_image = vec![CANARY; PAGE];
    let code_bytes = bytes(code);
    code_image[0x100..0x100 + code_bytes.len()].copy_from_slice(&code_bytes);
    let descriptors: Vec<_> = descriptor_users
        .iter()
        .map(|user| (*user, data[*user as usize / 4]))
        .collect();
    let command_bytes = commands(base + 0x10100, &descriptors, users, 64);
    command_image[0x1000..0x1000 + command_bytes.len()].copy_from_slice(&command_bytes);
    for index in 0..4 {
        memory
            .write(base + 0x20000 + index * PAGE as u64, &vec![CANARY; PAGE])
            .unwrap();
    }
    memory.write(base, &command_image).unwrap();
    memory.write(base + 0x10000, &code_image).unwrap();
    for address in [base, base + 0x10000] {
        owner
            .protect(
                address,
                PAGE as u64,
                NativePermissions::READ.with_gpu(true, false),
            )
            .unwrap();
    }
    Fixture {
        owner,
        root: NativeRange {
            address: base + 0x1000,
            size: command_bytes.len() as u64,
        },
        data,
        command_page: base,
        code_page: base + 0x10000,
        command_image,
        code_image,
    }
}
struct Observed<'a, 'owner> {
    source: CommandMemory<'a, 'owner>,
    snapshots: Cell<usize>,
}
impl compute::Memory for Observed<'_, '_> {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), compute::MemoryError> {
        compute::Memory::read(&self.source, address, output)
    }
    fn preflight_buffers(
        &self,
        buffers: &[compute::resources::BufferSpan],
        work: u64,
    ) -> Result<u64, compute::Error> {
        compute::Memory::preflight_buffers(&self.source, buffers, work)
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), compute::MemoryError> {
        self.snapshots.set(self.snapshots.get() + 1);
        compute::Memory::snapshot_buffer(&self.source, address, output, read, written)
    }
}

#[test]
fn all_owned_permissions_precede_first_resource_snapshot() {
    let mut fixture = fixture(&stores(), 16, &[0, 4, 8, 12], Alias::None);
    // S4 is the LAST first-use resource in this original shader.
    fixture
        .owner
        .protect(
            fixture.data[1] - 0x100,
            PAGE as u64,
            NativePermissions::READ_WRITE.with_gpu(true, false),
        )
        .unwrap();
    let memory = fixture.owner.pause().unwrap();
    let commands = [compute::CommandRange {
        address: fixture.root.address,
        size: fixture.root.size,
    }];
    let source = Observed {
        source: CommandMemory {
            paused: &memory,
            commands: &commands,
        },
        snapshots: Cell::new(0),
    };
    let result = compute::prepare(
        &fixture.command_image[0x1000..0x1000 + fixture.root.size as usize],
        &source,
        &Limits::default(),
    );
    assert!(matches!(
        result,
        Err(compute::Error::Memory(compute::MemoryError::WriteDenied))
    ));
    assert_eq!(source.snapshots.get(), 0);
    for address in fixture.data {
        let mut actual = [0; 64];
        memory.read(address, &mut actual).unwrap();
        assert_eq!(actual, [CANARY; 64]);
    }
}

#[test]
fn different_virtual_resource_and_command_aliases_refuse_before_any_snapshot() {
    for alias in [Alias::Resource, Alias::Command] {
        let mut fixture = fixture(&stores(), 16, &[0, 4, 8, 12], alias);
        let memory = fixture.owner.pause().unwrap();
        let commands = [compute::CommandRange {
            address: fixture.root.address,
            size: fixture.root.size,
        }];
        let source = Observed {
            source: CommandMemory {
                paused: &memory,
                commands: &commands,
            },
            snapshots: Cell::new(0),
        };
        let mut before = [0; 64];
        memory.read(fixture.data[0], &mut before).unwrap();
        assert!(matches!(
            compute::prepare(
                &fixture.command_image[0x1000..0x1000 + fixture.root.size as usize],
                &source,
                &Limits::default()
            ),
            Err(compute::Error::Memory(_))
        ));
        assert_eq!(source.snapshots.get(), 0);
        let mut after = [0; 64];
        memory.read(fixture.data[0], &mut after).unwrap();
        assert_eq!(after, before);
    }
}

#[test]
fn full_multi_output_transaction_budget_is_admitted_before_any_publication() {
    // This is an owned CPU transaction contract test, not GPU-result evidence.
    // Four sixteen-page outputs make the transaction's cross-output walk larger
    // than a single repetition of the resource/command exclusion proofs.
    const EXTENT: usize = 16 * PAGE;
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x180000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let permissions = NativePermissions::READ_WRITE.with_gpu(true, true);
    owner.map_at(base, PAGE as u64, permissions).unwrap();
    owner
        .map_at(base + 0x10000, PAGE as u64, permissions)
        .unwrap();
    let data: [u64; 4] = std::array::from_fn(|index| base + 0x40000 + index as u64 * EXTENT as u64);
    for address in data {
        owner.map_at(address, EXTENT as u64, permissions).unwrap();
    }
    let mut memory = owner.pause().unwrap();
    for address in data {
        memory.write(address, &vec![CANARY; EXTENT]).unwrap();
    }
    let code = base + 0x10100;
    memory.write(code, &bytes(&stores())).unwrap();
    let descriptors: Vec<_> = data
        .into_iter()
        .enumerate()
        .map(|(index, address)| (index as u32 * 4, address))
        .collect();
    let commands = commands(code, &descriptors, 16, EXTENT as u32);
    memory.write(base + 0x1000, &commands).unwrap();
    let roots = [compute::CommandRange {
        address: base + 0x1000,
        size: commands.len() as u64,
    }];
    let prepared = compute::prepare(
        &commands,
        &CommandMemory {
            paused: &memory,
            commands: &roots,
        },
        &Limits::default(),
    )
    .unwrap();
    let budget = output_transaction_work(prepared.buffers()).unwrap();
    let mut payloads: Vec<_> = prepared
        .buffers()
        .iter()
        .map(|buffer| buffer.input().to_vec())
        .collect();
    for (index, payload) in payloads.iter_mut().enumerate() {
        payload[..4].copy_from_slice(&(index as u32 + 17).to_le_bytes());
    }
    let outputs: Vec<_> = prepared
        .buffers()
        .iter()
        .zip(&payloads)
        .map(|(buffer, payload)| NativeMemoryWrite {
            address: buffer.address(),
            bytes: payload,
        })
        .collect();
    let mut limits = NativeTransactionLimits {
        max_outputs: 4,
        max_output_bytes: (EXTENT * 4) as u64,
        max_preflight_steps: budget - 1,
        ..Default::default()
    };
    assert!(matches!(
        memory.apply_gpu_writes(&outputs, &limits),
        Err(NativeMemoryError::BudgetExceeded)
    ));
    for address in data {
        let mut before = vec![0; EXTENT];
        memory.read(address, &mut before).unwrap();
        assert_eq!(before, vec![CANARY; EXTENT]);
    }
    limits.max_preflight_steps = budget;
    memory.apply_gpu_writes(&outputs, &limits).unwrap();
    for (output, expected) in outputs.iter().zip(&payloads) {
        let mut actual = vec![0; EXTENT];
        memory.read(output.address, &mut actual).unwrap();
        assert_eq!(&actual, expected);
    }
}

#[test]
#[ignore = "requires a real Vulkan device; run under the reviewed hardware gate"]
fn actual_zero_readonly_two_and_four_direct_resources_fence_and_publish_exact_outputs() {
    let mut actual_dispatches = 0;
    for case in 0..4 {
        let (code, users, descriptors) = match case {
            0 => (vec![0xbf81_0000], 0, vec![]),
            1 => (readonly(), 12, vec![8]),
            2 => (copy(), 12, vec![4, 8]),
            _ => (stores(), 16, vec![0, 4, 8, 12]),
        };
        let mut fixture = fixture(&code, users, &descriptors, Alias::None);
        if case == 1 || case == 2 {
            fixture
                .owner
                .protect(
                    fixture.data[2] - 0x100,
                    PAGE as u64,
                    NativePermissions::READ_WRITE.with_gpu(true, false),
                )
                .unwrap();
        }
        if case == 2 {
            fixture
                .owner
                .protect(
                    fixture.data[1] - 0x100,
                    PAGE as u64,
                    NativePermissions::READ_WRITE.with_gpu(false, true),
                )
                .unwrap();
        }
        if case == 3 {
            for address in fixture.data {
                fixture
                    .owner
                    .protect(
                        address - 0x100,
                        PAGE as u64,
                        NativePermissions::READ_WRITE.with_gpu(false, true),
                    )
                    .unwrap();
            }
        }
        let mut expected: [Vec<u8>; 4] = std::array::from_fn(|_| vec![CANARY; PAGE]);
        let mut memory = fixture.owner.pause().unwrap();
        if case == 2 {
            let seed = 0x1020_3040u32.to_le_bytes();
            memory.write(fixture.data[2], &seed).unwrap();
            expected[2][0x100..0x104].copy_from_slice(&seed);
            expected[1][0x100..0x104].copy_from_slice(&seed);
        }
        if case == 3 {
            // The compiler array is in first-use order [s12,s0,s8,s4], not
            // register order. A reordered descriptor upload changes these words.
            for (page, value) in expected.iter_mut().zip([2u32, 4, 3, 1]) {
                page[0x100..0x104].copy_from_slice(&value.to_le_bytes());
            }
        }
        let report = execute_compute(
            &mut memory,
            fixture.root,
            &Limits::default(),
            &GpuLimits::default(),
        )
        .expect("original direct-resource dispatch must execute and fence on the actual GPU");
        assert_eq!(report.preparation.invocations, 1);
        assert_eq!(report.buffer_bytes, [0, 64, 128, 256][case]);
        assert_eq!(report.written_bytes, [0, 0, 64, 256][case]);
        assert!(report.gpu.commands >= 3);
        assert!(report.gpu.shader_words > 0);
        if case == 0 {
            assert_eq!(report.gpu.device_bytes, 0);
        }
        for (address, expected) in fixture.data.into_iter().zip(expected) {
            let mut actual = vec![0; PAGE];
            memory.read(address - 0x100, &mut actual).unwrap();
            assert_eq!(actual, expected);
        }
        for (address, expected) in [
            (fixture.command_page, &fixture.command_image),
            (fixture.code_page, &fixture.code_image),
        ] {
            let mut actual = vec![0; PAGE];
            memory.read(address, &mut actual).unwrap();
            assert_eq!(&actual, expected);
        }
        actual_dispatches += 1;
    }
    assert_eq!(actual_dispatches, 4);
    println!("PENTAX_ACTUAL_DIRECT_COMPUTE_ZERO_READONLY_TWO_FOUR_FENCED_V1");
}
